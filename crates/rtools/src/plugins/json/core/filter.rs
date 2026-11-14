//! JavaScript filter engine (spec §5.2): evaluates `"this" + expr` against the
//! document inside QuickJS on a dedicated worker thread.

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Function, Runtime};

use super::JsonError;

/// Time budget for one user expression; parsing the document is not counted.
pub const FILTER_TIMEOUT: Duration = Duration::from_secs(2);
/// QuickJS heap limit.
const FILTER_MEMORY_LIMIT: usize = 1 << 30;

/// Installs three non-writable globals whose closures capture the builtins once,
/// so user code that reassigns `JSON.stringify`, `String`, `eval` or
/// `Reflect.apply` cannot break later evaluations:
/// - `__rtoolsLoad(src)` stores the document source;
/// - `__rtoolsPrepare()` parses a fresh document, so side effects of the previous
///   expression (`.reverse()`, assignments) never leak into the next result;
/// - `__rtoolsRun(code)` evaluates `code` with `this` bound to the document.
///
/// The prelude is sloppy mode on purpose: `var eval` binds the captured `%eval%`,
/// which keeps `eval(code)` a direct eval that sees the function's `this`.
/// Captured names are prefixed because direct eval can see the enclosing scope.
const PRELUDE: &str = r#"(function ($$rtoolsParse, $$rtoolsStringify, $$rtoolsString, $$rtoolsApply, $$rtoolsEval, $$rtoolsDefine) {
    var $$rtoolsSource = '';
    var $$rtoolsDoc;
    var $$rtoolsInstall = function (name, value) {
        $$rtoolsDefine(globalThis, name, { value: value, writable: false, configurable: false });
    };
    $$rtoolsInstall('__rtoolsLoad', function (src) { $$rtoolsSource = src; });
    $$rtoolsInstall('__rtoolsPrepare', function () { $$rtoolsDoc = $$rtoolsParse($$rtoolsSource); });
    $$rtoolsInstall('__rtoolsRun', function ($$rtoolsCode) {
        var eval = $$rtoolsEval;
        var result = $$rtoolsApply(function () { return eval($$rtoolsCode); }, $$rtoolsDoc, []);
        if (result === undefined) return 'undefined';
        var text;
        try { text = $$rtoolsStringify(result, null, 4); } catch ($$rtoolsError) { text = undefined; }
        return text === undefined ? $$rtoolsString(result) : text;
    });
})(JSON.parse, JSON.stringify, String, Reflect.apply, eval, Object.defineProperty)"#;

/// Work sent to the QuickJS thread.
enum Job {
    SetDocument(Arc<str>),
    Evaluate(String, oneshot::Sender<Result<String, JsonError>>),
}

/// Handle to the filter worker; dropping it stops the worker thread.
pub struct FilterEngine {
    jobs: mpsc::Sender<Job>,
}

impl FilterEngine {
    /// Starts the worker thread (expression timeout `FILTER_TIMEOUT`) and waits
    /// until QuickJS is initialised.
    pub fn spawn() -> Result<Self, JsonError> {
        let (jobs, inbox) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("rtools-json-filter".into())
            .spawn(move || match Worker::new() {
                Ok(worker) => {
                    // The caller blocks on this message, so the send cannot fail.
                    let _ = ready_tx.send(Ok(()));
                    worker.run(inbox);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            })
            // Without a worker thread there is no engine at all.
            .map_err(|_| JsonError::EngineStopped)?;
        ready_rx.recv().map_err(|_| JsonError::EngineStopped)??;
        Ok(Self { jobs })
    }

    /// Replaces the source text; it is parsed lazily on the next evaluation.
    pub fn set_document(&self, text: Arc<str>) {
        // A stopped engine has no document to update; evaluations report the failure.
        let _ = self.jobs.send(Job::SetDocument(text));
    }

    /// Evaluates `"this" + expr` against the document.
    ///
    /// The job is queued immediately; the future only waits for its result.
    pub fn evaluate(
        &self,
        expr: &str,
    ) -> impl Future<Output = Result<String, JsonError>> + Send + 'static {
        let (reply, result) = oneshot::channel();
        // If the worker is gone the job (and its reply sender) is dropped here,
        // so the receiver resolves to `Canceled` and maps to `EngineStopped`.
        let _ = self.jobs.send(Job::Evaluate(format!("this{expr}"), reply));
        async move { result.await? }
    }
}

/// State of the current document inside QuickJS.
enum Document {
    /// Text received but not yet handed to QuickJS.
    Pending(Arc<str>),
    /// Handed to `__rtoolsLoad`.
    Ready,
    /// Parsing failed; repeated for every evaluation until a new document arrives.
    Invalid(JsonError),
}

/// QuickJS state owned by the worker thread.
struct Worker {
    _runtime: Runtime,
    context: Context,
    /// When set, the interrupt handler aborts execution after this instant.
    deadline: Rc<Cell<Option<Instant>>>,
    /// Set by the interrupt handler when it aborted execution.
    timed_out: Rc<Cell<bool>>,
    document: Document,
}

impl Worker {
    /// Creates the runtime with its memory limit and timeout interrupt handler.
    fn new() -> Result<Self, JsonError> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(FILTER_MEMORY_LIMIT);
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let timed_out = Rc::new(Cell::new(false));
        let (handler_deadline, handler_timed_out) = (Rc::clone(&deadline), Rc::clone(&timed_out));
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let expired = handler_deadline
                .get()
                .is_some_and(|deadline| Instant::now() >= deadline);
            if expired {
                handler_timed_out.set(true);
            }
            expired
        })));
        let context = Context::full(&runtime)?;
        // Sloppy mode, matching the original plugin; see `PRELUDE` for why it matters.
        let mut options = EvalOptions::default();
        options.strict = false;
        context.with(|ctx| {
            ctx.eval_with_options::<(), _>(PRELUDE, options)
                .map_err(|err| script_error(&ctx, err))
        })?;
        Ok(Self {
            _runtime: runtime,
            context,
            deadline,
            timed_out,
            document: Document::Pending(Arc::from("")),
        })
    }

    /// Processes jobs until every `FilterEngine` handle is dropped.
    fn run(mut self, inbox: mpsc::Receiver<Job>) {
        // ponytail: jobs run sequentially; stale evaluations are discarded by the UI
        // sequence number, add queue draining if typing outpaces evaluation.
        for job in inbox {
            match job {
                Job::SetDocument(text) => self.document = Document::Pending(text),
                Job::Evaluate(code, reply) => {
                    // The caller may have stopped waiting; then the result is simply dropped.
                    let _ = reply.send(self.evaluate(code));
                }
            }
        }
    }

    /// Rebuilds the document from its source, then runs `code` under the deadline.
    fn evaluate(&mut self, code: String) -> Result<String, JsonError> {
        self.reset_document()?;
        let (deadline, timed_out) = (&self.deadline, &self.timed_out);
        self.context.with(|ctx| {
            let run: Function = ctx.globals().get("__rtoolsRun")?;
            timed_out.set(false);
            deadline.set(Some(Instant::now() + FILTER_TIMEOUT));
            let result = run.call::<_, String>((code,));
            deadline.set(None);
            result.map_err(|err| {
                if timed_out.get() {
                    // The interrupt raises an exception; drop it so the context stays clean.
                    let _ = ctx.catch();
                    JsonError::Timeout
                } else {
                    script_error(&ctx, err)
                }
            })
        })
    }

    /// Loads a pending source, then parses a fresh document (`__rtoolsPrepare`)
    /// without a deadline: the cost depends only on the document size and it
    /// cannot loop forever.
    fn reset_document(&mut self) -> Result<(), JsonError> {
        match &self.document {
            Document::Invalid(err) => return Err(err.clone()),
            Document::Pending(text) => {
                let stored = self.context.with(|ctx| {
                    let load: Function = ctx.globals().get("__rtoolsLoad")?;
                    load.call::<_, ()>((text.as_ref(),))
                });
                stored?;
                self.document = Document::Ready;
            }
            Document::Ready => {}
        }
        // ponytail: integers beyond 2^53 lose precision in JSON.parse, upgrade with a
        // BigInt-aware parser.
        let parsed = self.context.with(|ctx| {
            let prepare: Function = ctx.globals().get("__rtoolsPrepare")?;
            prepare
                .call::<_, ()>(())
                .map_err(|err| script_error(&ctx, err))
        });
        if let Err(err) = &parsed {
            self.document = Document::Invalid(err.clone());
        }
        parsed
    }
}

/// Turns a QuickJS failure into a `Script` error, reading the pending exception if any.
fn script_error(ctx: &Ctx<'_>, err: rquickjs::Error) -> JsonError {
    if !err.is_exception() {
        return JsonError::from(err);
    }
    let thrown = ctx.catch();
    // `Exception`'s Display always says "Error"; read the real name (SyntaxError, TypeError…).
    let message = match thrown.as_exception() {
        Some(exception) => {
            let name = exception
                .get::<_, String>("name")
                .unwrap_or_else(|_| "Error".into());
            format!("{name}: {}", exception.message().unwrap_or_default())
        }
        None => thrown
            .get::<rquickjs::Coerced<String>>()
            .map(|text| text.0)
            .unwrap_or_else(|_| "unknown error".into()),
    };
    JsonError::Script(message.lines().next().unwrap_or_default().to_string())
}
