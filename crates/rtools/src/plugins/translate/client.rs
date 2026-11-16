//! Rig owns DeepSeek encoding and stream decoding; this client enforces application limits.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_io::Timer;
use futures::{
    FutureExt as _, SinkExt as _, StreamExt as _,
    channel::mpsc,
    future::{Either, select},
};
use gpui_kit::Global;
use rig_core::{
    Model, ProviderError,
    catalog::{Catalog, ModelFacts},
    completion::{CompletionRequest, FinishReason},
    http_client::{
        self, Bytes, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
        Uri,
    },
    providers::openai::wire::{Chat, DEEPSEEK, OpenAIConfig},
    streaming::{Item, StreamEvent},
    wasm_compat::WasmCompatSend,
};
use rig_reqwest::{ReqwestClient, reqwest};
use tracing::instrument::WithSubscriber as _;

use super::{
    core::{Completion, OUTPUT_LIMIT, TranslateError, TranslationRequest},
    credentials::Credential,
};

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONTENT_TIMEOUT: Duration = Duration::from_secs(60);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(180);
const WIRE_LIMIT: usize = 16 * 1024 * 1024;

/// Clones share Rig's configured HTTP pool and its executor-compatible reactor.
#[derive(Clone)]
pub struct DeepSeekClient {
    http: ReqwestClient,
    /// A request-local exact URL; the global pool has no endpoint attached.
    endpoint: Option<Uri>,
}

impl Global for DeepSeekClient {}

impl DeepSeekClient {
    /// Creates a TLS-verified transport that never forwards credentials through redirects.
    pub fn new() -> Result<Self, TranslateError> {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|http| Self {
                http: ReqwestClient::from(http),
                endpoint: None,
            })
            .map_err(|_| {
                tracing::error!("initializing translation HTTP client failed");
                TranslateError::Network
            })
    }

    /// Streams bounded text deltas; dropping this future cancels the Rig stream and transport.
    pub async fn translate(
        &self,
        request: TranslationRequest,
        key: Credential,
        sender: mpsc::Sender<String>,
    ) -> Result<Completion, TranslateError> {
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        tracing::info!(
            "translation request {} started: host {}, direction {:?}, scenario {:?}, {} characters",
            id,
            request.endpoint.host(),
            request.direction,
            request.scenario,
            request.input.chars().count()
        );
        // Rig may log raw provider error bodies; only our safe lifecycle events may escape.
        let result = self
            .stream(&request, key, sender)
            .with_subscriber(tracing::subscriber::NoSubscriber::default())
            .await;
        match &result {
            Ok(completion) => tracing::info!(
                "translation request {} finished {:?} in {} ms",
                id,
                completion,
                started.elapsed().as_millis()
            ),
            Err(error) => tracing::warn!(
                "translation request {} failed after {} ms: {}",
                id,
                started.elapsed().as_millis(),
                error
            ),
        }
        result
    }

    /// Consumes Rig's typed content events, ignoring reasoning and rejecting incomplete replies.
    async fn stream(
        &self,
        request: &TranslationRequest,
        key: Credential,
        mut sender: mpsc::Sender<String>,
    ) -> Result<Completion, TranslateError> {
        let mut transport = self.clone();
        transport.endpoint = Some(
            request
                .endpoint
                .as_str()
                .parse()
                .map_err(|_| TranslateError::Endpoint)?,
        );
        let model =
            Model::new(request.wire(key), transport).with_idle_timeout(Some(CONTENT_TIMEOUT));
        let mut stream = model
            .stream(CompletionRequest::from(request))
            .map_err(TranslateError::from)?;
        let mut last_content = Instant::now();
        let started = last_content;
        let mut output_bytes = 0;
        loop {
            let deadline = (last_content + CONTENT_TIMEOUT).min(started + TOTAL_TIMEOUT);
            let item = match select(
                stream.next().boxed(),
                futures::FutureExt::boxed(Timer::at(deadline)),
            )
            .await
            {
                Either::Left((item, _)) => item,
                Either::Right(_) => return Err(TranslateError::Timeout),
            };
            let Some(item) = item else {
                break;
            };
            if let Item::Event(StreamEvent::Text { text, .. }) =
                item.map_err(TranslateError::from)?
            {
                if text.is_empty() {
                    continue;
                }
                output_bytes += text.len();
                if output_bytes > OUTPUT_LIMIT {
                    return Err(TranslateError::OutputLimit);
                }
                last_content = Instant::now();
                sender
                    .send(text)
                    .await
                    .map_err(|_| TranslateError::Cancelled)?;
            }
        }
        let response = stream.finish().await.map_err(TranslateError::from)?;
        if output_bytes == 0 {
            return Err(TranslateError::EmptyResponse);
        }
        if response.error.is_some() {
            return Err(TranslateError::Protocol);
        }
        match response.finish_reason() {
            Some(FinishReason::Length) => Ok(Completion::Truncated),
            Some(FinishReason::Stop) | None => Ok(Completion::Complete),
            _ => Err(TranslateError::Protocol),
        }
    }
}

impl HttpClientExt for DeepSeekClient {
    /// Delegates buffered HTTP operations to Rig's transport without adding retries.
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.http.send(request)
    }

    /// Delegates the transport contract's unused multipart path to the same client.
    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.http.send_multipart(request)
    }

    /// Caps wire bytes before Rig buffers any oversized or unterminated SSE event.
    async fn send_streaming<T>(
        &self,
        mut request: Request<T>,
    ) -> http_client::Result<StreamingResponse>
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        // Rig owns protocol paths; the user preference deliberately names the full URL.
        if let Some(endpoint) = &self.endpoint {
            *request.uri_mut() = endpoint.clone();
        }
        let response = self.http.send_streaming(request).await?;
        let (parts, body) = response.into_parts();
        let bounded = body.scan(0usize, |received, chunk| {
            if let Ok(bytes) = &chunk {
                *received = received.saturating_add(bytes.len());
            }
            let chunk = if *received > WIRE_LIMIT {
                Err(http_client::Error::instance(TranslateError::OutputLimit))
            } else {
                chunk
            };
            futures::future::ready(Some(chunk))
        });
        Ok(Response::from_parts(parts, Box::pin(bounded)))
    }
}

impl TranslationRequest {
    /// Builds the DeepSeek wire with explicit facts, without loading the model catalog.
    pub fn wire(&self, key: Credential) -> Chat {
        OpenAIConfig::with_key(&DEEPSEEK, key.secret)
            .chat(&self.model)
            // Rig 0.46's built-in catalog conflicts with serde_json/arbitrary_precision.
            // Empty explicit facts also avoid imposing a model allowlist on custom endpoints.
            .with_facts(ModelFacts::default().with_catalog(&Catalog::default()))
    }
}

impl From<ProviderError> for TranslateError {
    /// Reduces provider errors to safe categories without exposing bodies or credentials.
    fn from(error: ProviderError) -> Self {
        if let Some(status) = error
            .provider_response_status()
            .filter(|status| !status.is_success())
        {
            return Self::Http(status.as_u16());
        }
        match error {
            ProviderError::Truncated => Self::Interrupted,
            ProviderError::Stalled(_) => Self::Timeout,
            ProviderError::Http(error) => {
                if let Some(status) = error.non_success_status() {
                    return Self::Http(status.as_u16());
                }
                if let http_client::Error::Instance(inner) = &*error {
                    if let Some(error) = inner.downcast_ref::<TranslateError>() {
                        return error.clone();
                    }
                    if inner
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(reqwest::Error::is_timeout)
                    {
                        return Self::Timeout;
                    }
                }
                Self::Network
            }
            _ => Self::Protocol,
        }
    }
}
