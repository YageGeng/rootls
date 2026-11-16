//! The settings window (spec §4.6, §5.7): a Zed-style sidebar with search and
//! page navigation, and one row per setting with a fixed-width control lane.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, Theme, ThemeMode,
    ThemeRegistry, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use std::ops::RangeInclusive;

use super::{Appearance, FONT_SIZES, Language, Settings, SettingsStore, WINDOW_RADII};
use crate::autostart::AutostartStore;
use crate::ui::Choice;

actions!(settings_window, [CloseSettings]);

/// Key context of the settings window.
const CONTEXT: &str = "SettingsWindow";

/// Installs the settings window key bindings.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some(CONTEXT))]);
}

/// Font dropdown options; `None` follows the theme.
type FontChoices = SearchableVec<Choice<Option<SharedString>>>;
/// Language dropdown options.
type LanguageChoices = Vec<Choice<Language>>;
/// Color modes shared by the interface and editors.
type AppearanceChoices = Vec<Choice<Appearance>>;
/// Preset names filtered by light or dark mode.
type ThemeChoices = SearchableVec<Choice<SharedString>>;

/// A page of the sidebar navigation.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Page {
    General,
    Appearance,
    Language,
    /// Options of the JSON editor plugin.
    Json,
    Translate,
}

impl Page {
    /// Every page, in sidebar order: the app first, then plugins.
    const ALL: [Page; 5] = [
        Page::General,
        Page::Appearance,
        Page::Language,
        Page::Json,
        Page::Translate,
    ];

    /// Translation keys of the page title and subtitle.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Page::General => ("settings.general", "settings.general_subtitle"),
            Page::Appearance => ("settings.appearance", "settings.appearance_subtitle"),
            Page::Language => ("settings.language", "settings.language_subtitle"),
            Page::Json => ("settings.json", "settings.json_subtitle"),
            Page::Translate => ("translate.title", "translate.settings_subtitle"),
        }
    }
}

/// One setting row; rows are listed in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Autostart,
    Appearance,
    LightTheme,
    DarkTheme,
    UiFont,
    MonoFont,
    FontSize,
    WindowRadius,
    Language,
    SoftWrap,
}

impl Row {
    /// Every row, in display order.
    const ALL: [Row; 10] = [
        Row::Autostart,
        Row::Appearance,
        Row::LightTheme,
        Row::DarkTheme,
        Row::UiFont,
        Row::MonoFont,
        Row::FontSize,
        Row::WindowRadius,
        Row::Language,
        Row::SoftWrap,
    ];

    /// The page the row belongs to.
    fn page(self) -> Page {
        match self {
            Row::Autostart => Page::General,
            Row::Appearance
            | Row::LightTheme
            | Row::DarkTheme
            | Row::UiFont
            | Row::MonoFont
            | Row::FontSize
            | Row::WindowRadius => Page::Appearance,
            Row::Language => Page::Language,
            Row::SoftWrap => Page::Json,
        }
    }

    /// Translation keys of the row's section, title and description.
    fn keys(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Row::Autostart => (
                "settings.startup",
                "settings.autostart",
                "settings.autostart_detail",
            ),
            Row::Appearance => ("settings.colors", "settings.theme", "settings.theme_detail"),
            Row::LightTheme => (
                "settings.colors",
                "settings.light_theme",
                "settings.light_theme_detail",
            ),
            Row::DarkTheme => (
                "settings.colors",
                "settings.dark_theme",
                "settings.dark_theme_detail",
            ),
            Row::UiFont => (
                "settings.fonts",
                "settings.ui_font",
                "settings.ui_font_detail",
            ),
            Row::MonoFont => (
                "settings.fonts",
                "settings.mono_font",
                "settings.mono_font_detail",
            ),
            Row::FontSize => (
                "settings.scale",
                "settings.font_size",
                "settings.font_size_detail",
            ),
            Row::WindowRadius => (
                "settings.window",
                "settings.window_radius",
                "settings.window_radius_detail",
            ),
            Row::Language => (
                "settings.display",
                "settings.interface_language",
                "settings.interface_language_detail",
            ),
            Row::SoftWrap => (
                "settings.editor",
                "settings.soft_wrap",
                "settings.soft_wrap_detail",
            ),
        }
    }

    /// Whether the localized title or description contains the lowercase `query`.
    fn matches(self, query: &str) -> bool {
        let (_, title, detail) = self.keys();
        t!(title).to_lowercase().contains(query) || t!(detail).to_lowercase().contains(query)
    }
}

/// Points a font dropdown at the setting it edits.
type FontField = fn(&mut Settings) -> &mut Option<String>;
/// Points a number box at the setting it edits.
type NumberField = fn(&mut Settings) -> &mut f32;
/// Points a preset dropdown at its persisted theme name.
type ThemeField = fn(&mut Settings) -> &mut String;

/// The settings page: sidebar plus the rows of the current page or search.
pub struct SettingsWindow {
    focus: FocusHandle,
    search: Entity<InputState>,
    page: Page,
    translate: Option<Entity<super::translate::TranslateSettingsView>>,
    appearance: Entity<SelectState<AppearanceChoices>>,
    light_theme: Entity<SelectState<ThemeChoices>>,
    dark_theme: Entity<SelectState<ThemeChoices>>,
    ui_font: Entity<SelectState<FontChoices>>,
    mono_font: Entity<SelectState<FontChoices>>,
    font_size: Entity<InputState>,
    window_radius: Entity<InputState>,
    language: Entity<SelectState<LanguageChoices>>,
    /// Installed font families, sorted; enumerated once per window.
    font_names: Vec<SharedString>,
    /// The settings file as shown above the page, e.g. `~/.config/rtools/settings.toml`.
    path_label: SharedString,
    /// Locale the labels were built in; a change rebuilds the dropdown options.
    locale: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    /// Builds the page from the settings in effect.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.global::<SettingsStore>();
        let (font_size, window_radius) = (store.settings.font_size, store.settings.window_radius);
        let path = store.file.path();
        let path_label = match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok()) {
            Some(relative) => format!("~/{}", relative.display()),
            None => path.display().to_string(),
        }
        .into();
        let mut font_names = cx.text_system().all_font_names();
        font_names.sort_unstable();
        font_names.dedup();

        // Options, selections and placeholders are filled by `relabel` below.
        let search = cx.new(|cx| InputState::new(window, cx));
        let appearance = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let presets = |mode, window: &mut Window, cx: &mut Context<Self>| {
            let items: Vec<_> = ThemeRegistry::global(cx)
                .sorted_themes()
                .into_iter()
                .filter(|theme| theme.mode == mode)
                .map(|theme| Choice {
                    label: theme.name.clone(),
                    value: theme.name.clone(),
                })
                .collect();
            let theme = Theme::global(cx);
            let selected = if mode == ThemeMode::Dark {
                theme.dark_theme.name.clone()
            } else {
                theme.light_theme.name.clone()
            };
            cx.new(|cx| {
                let mut state =
                    SelectState::new(SearchableVec::new(items), None, window, cx).searchable(true);
                state.set_selected_value(&selected, window, cx);
                state
            })
        };
        let light_theme = presets(ThemeMode::Light, window, cx);
        let dark_theme = presets(ThemeMode::Dark, window, cx);
        let ui_font = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let mono_font = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let number = |value: f32,
                      range: &RangeInclusive<f32>,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .step(1.)
                    .min(f64::from(*range.start()))
                    .max(f64::from(*range.end()))
            })
        };
        let font_size = number(font_size, &FONT_SIZES, window, cx);
        let window_radius = number(window_radius, &WINDOW_RADII, window, cx);
        let language = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));

        let font_fields: [(&Entity<SelectState<FontChoices>>, FontField); 2] = [
            (&ui_font, |settings| &mut settings.ui_font),
            (&mono_font, |settings| &mut settings.mono_font),
        ];
        let mut subscriptions: Vec<Subscription> = font_fields
            .into_iter()
            .map(|(state, field)| {
                cx.subscribe(state, move |_, _, event: &SelectEvent<FontChoices>, cx| {
                    if let SelectEvent::Confirm(Some(family)) = event {
                        let family = family.as_ref().map(ToString::to_string);
                        SettingsStore::update(cx, |settings| *field(settings) = family);
                    }
                })
            })
            .collect();
        let theme_fields: [(&Entity<SelectState<ThemeChoices>>, ThemeField); 2] = [
            (&light_theme, |settings| &mut settings.light_theme),
            (&dark_theme, |settings| &mut settings.dark_theme),
        ];
        subscriptions.extend(theme_fields.into_iter().map(|(state, field)| {
            cx.subscribe(state, move |_, _, event: &SelectEvent<ThemeChoices>, cx| {
                if let SelectEvent::Confirm(Some(name)) = event {
                    SettingsStore::update(cx, |settings| *field(settings) = name.to_string());
                }
            })
        }));
        let number_fields: [(&Entity<InputState>, RangeInclusive<f32>, NumberField); 2] = [
            (&font_size, FONT_SIZES, |settings| &mut settings.font_size),
            (&window_radius, WINDOW_RADII, |settings| {
                &mut settings.window_radius
            }),
        ];
        subscriptions.extend(number_fields.into_iter().map(|(state, range, field)| {
            cx.subscribe(state, move |_, state, event: &InputEvent, cx| {
                // Partial input such as "1" while typing "18" is ignored until it is in range.
                if matches!(event, InputEvent::Change)
                    && let Ok(value) = state.read(cx).value().trim().parse::<f32>()
                    && range.contains(&value)
                {
                    SettingsStore::update(cx, |settings| *field(settings) = value);
                }
            })
        }));
        subscriptions.extend([
            cx.observe_global::<AutostartStore>(|_, cx| cx.notify()),
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    AutostartStore::request(None, cx);
                }
            }),
            cx.subscribe(
                &appearance,
                |_, _, event: &SelectEvent<AppearanceChoices>, cx| {
                    if let SelectEvent::Confirm(Some(appearance)) = event {
                        let appearance = *appearance;
                        SettingsStore::update(cx, |settings| settings.appearance = appearance);
                    }
                },
            ),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(
                &language,
                |_, _, event: &SelectEvent<LanguageChoices>, cx| {
                    if let SelectEvent::Confirm(Some(language)) = event {
                        let language = *language;
                        SettingsStore::update(cx, |settings| settings.language = language);
                    }
                },
            ),
            // Redraws switches and relabels the options after a language change.
            cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
                this.relabel(window, cx);
                // Reloading the settings file also updates an already-open dropdown.
                let appearance = cx.global::<SettingsStore>().settings.appearance;
                this.appearance.update(cx, |state, cx| {
                    state.set_selected_value(&appearance, window, cx);
                });
                // Show the effective fallback when a hand-edited theme name is invalid.
                let theme = Theme::global(cx);
                let selected = [
                    (&this.light_theme, theme.light_theme.name.clone()),
                    (&this.dark_theme, theme.dark_theme.name.clone()),
                ];
                for (state, name) in selected {
                    state.update(cx, |state, cx| {
                        state.set_selected_value(&name, window, cx);
                    });
                }
                cx.notify();
            }),
        ]);

        let mut this = Self {
            focus: cx.focus_handle(),
            search,
            page: Page::General,
            translate: None,
            appearance,
            light_theme,
            dark_theme,
            ui_font,
            mono_font,
            font_size,
            window_radius,
            language,
            font_names: font_names.into_iter().map(Into::into).collect(),
            path_label,
            // No locale yet, so `relabel` fills everything in.
            locale: SharedString::default(),
            _subscriptions: subscriptions,
        };
        this.relabel(window, cx);
        AutostartStore::request(None, cx);
        this
    }

    /// Fills the localized dropdown options, their selections and the search
    /// placeholder whenever the locale differs from the one they were built in.
    fn relabel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locale: SharedString = rust_i18n::locale().to_string().into();
        if locale == self.locale {
            return;
        }
        self.locale = locale;
        let settings = cx.global::<SettingsStore>().settings.clone();
        let appearances = [
            (Appearance::System, "settings.theme_system"),
            (Appearance::Light, "settings.theme_light"),
            (Appearance::Dark, "settings.theme_dark"),
        ]
        .into_iter()
        .map(|(value, key)| Choice {
            label: t!(key).into(),
            value,
        })
        .collect();
        self.appearance.update(cx, |state, cx| {
            state.set_items(appearances, window, cx);
            state.set_selected_value(&settings.appearance, window, cx);
        });
        // "Theme default" first, then every installed family.
        let fonts: Vec<Choice<Option<SharedString>>> = std::iter::once(Choice {
            label: t!("settings.theme_font").into(),
            value: None,
        })
        .chain(self.font_names.iter().map(|name| Choice {
            label: name.clone(),
            value: Some(name.clone()),
        }))
        .collect();
        for (state, family) in [
            (&self.ui_font, settings.ui_font),
            (&self.mono_font, settings.mono_font),
        ] {
            let items = SearchableVec::new(fonts.clone());
            let family: Option<SharedString> = family.map(Into::into);
            state.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_value(&family, window, cx);
            });
        }
        let languages = [
            (Language::System, "settings.language_system"),
            (Language::ZhCn, "settings.language_zh_cn"),
            (Language::En, "settings.language_en"),
        ]
        .into_iter()
        .map(|(value, key)| Choice {
            label: t!(key).into(),
            value,
        })
        .collect();
        self.language.update(cx, |state, cx| {
            state.set_items(languages, window, cx);
            state.set_selected_value(&settings.language, window, cx);
        });
        self.search.update(cx, |state, cx| {
            state.set_placeholder(t!("settings.search"), window, cx);
        });
    }

    /// Shows a page, leaving any search.
    pub(crate) fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        if page == Page::General {
            AutostartStore::request(None, cx);
        }
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Opens the settings file in the system editor, writing it first if missing.
    fn edit_file(&mut self, cx: &mut Context<Self>) {
        let store = cx.global_mut::<SettingsStore>();
        let path = store.file.path().to_path_buf();
        if !path.exists()
            && let Err(err) = store.file.save(&store.settings)
        {
            tracing::warn!("creating settings file {} failed: {}", path.display(), err);
            return;
        }
        cx.open_with_system(&path);
    }

    /// Closes the window (Esc).
    fn close(&mut self, _: &CloseSettings, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    /// The control lane element of a row.
    fn control(&self, row: Row, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let select_font = |state: &Entity<SelectState<FontChoices>>| {
            Select::new(state)
                .appearance(false)
                .icon(IconName::ChevronsUpDown)
                .menu_width(px(280.))
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element()
        };
        let number = |state: &Entity<InputState>| {
            NumberInput::new(state)
                .appearance(false)
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element()
        };
        match row {
            Row::Autostart => h_flex()
                .justify_end()
                .child(
                    Switch::new("autostart")
                        .accessibility_label(t!("settings.autostart"))
                        .checked(
                            cx.global::<AutostartStore>()
                                .status
                                .is_some_and(|status| status.registered()),
                        )
                        .disabled(cx.global::<AutostartStore>().busy)
                        .on_click(|checked, _, cx| AutostartStore::request(Some(*checked), cx)),
                )
                .into_any_element(),
            Row::Appearance => Select::new(&self.appearance)
                .appearance(false)
                .icon(IconName::ChevronsUpDown)
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element(),
            Row::LightTheme | Row::DarkTheme => {
                let state = if row == Row::LightTheme {
                    &self.light_theme
                } else {
                    &self.dark_theme
                };
                Select::new(state)
                    .appearance(false)
                    .icon(IconName::ChevronsUpDown)
                    .menu_width(rems(20.))
                    .bg(theme.secondary)
                    .rounded(theme.radius)
                    .into_any_element()
            }
            Row::UiFont => select_font(&self.ui_font),
            Row::MonoFont => select_font(&self.mono_font),
            Row::FontSize => number(&self.font_size),
            Row::WindowRadius => number(&self.window_radius),
            Row::Language => Select::new(&self.language)
                .appearance(false)
                .icon(IconName::ChevronsUpDown)
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element(),
            Row::SoftWrap => {
                let soft_wrap = cx.global::<SettingsStore>().settings.json.soft_wrap;
                h_flex()
                    .justify_end()
                    .child(Switch::new("soft-wrap").checked(soft_wrap).on_click(
                        |checked, _, cx| {
                            let checked = *checked;
                            SettingsStore::update(cx, |settings| {
                                settings.json.soft_wrap = checked;
                            });
                        },
                    ))
                    .into_any_element()
            }
        }
    }

    /// Sidebar: search box, page navigation and the Esc hint.
    fn render_sidebar(
        &self,
        searching: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let nav = Page::ALL.map(|page| {
            let active = !searching && page == self.page;
            let (title, _) = page.keys();
            h_flex()
                .id(title)
                .h_8()
                .px_2()
                .gap_1p5()
                .rounded(theme.radius)
                .text_base()
                .cursor_pointer()
                .when(active, |this| this.bg(theme.secondary))
                .when(!active, |this| {
                    this.text_color(muted)
                        .hover(|style| style.bg(theme.secondary.opacity(0.5)))
                })
                .on_click(cx.listener(move |this, _, window, cx| this.open_page(page, window, cx)))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size_3p5()
                        .text_color(muted),
                )
                .child(t!(title).to_string())
        });
        v_flex()
            .w_64()
            .flex_none()
            .p_2p5()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(Input::new(&self.search).prefix(Icon::new(IconName::Search).text_color(muted)))
            .child(div().h_2())
            .children(nav)
            .child(div().flex_1())
            .child(
                h_flex()
                    .px_1()
                    .gap_2()
                    .children(Kbd::binding_for_action(
                        &CloseSettings,
                        Some(CONTEXT),
                        window,
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("settings.close").to_string()),
                    ),
            )
    }

    /// File path row, page title and the visible setting rows.
    fn render_content(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let danger = theme.danger;
        let mono_font = theme.mono_font_family.clone();
        let rows: Vec<Row> = Row::ALL
            .into_iter()
            .filter(|row| {
                if query.is_empty() {
                    row.page() == self.page
                } else {
                    row.matches(query)
                }
            })
            .collect();
        let (title, subtitle) = if query.is_empty() {
            let (title, subtitle) = self.page.keys();
            (t!(title), Some(t!(subtitle)))
        } else {
            (t!("settings.search_results"), None)
        };

        let mut body = v_flex();
        let mut last_section = None;
        for row in &rows {
            let (section, row_title, detail) = row.keys();
            if last_section != Some(section) {
                last_section = Some(section);
                body = body.child(
                    div()
                        .pt_6()
                        .pb_2()
                        .border_b_1()
                        .border_color(border)
                        .text_sm()
                        .text_color(muted)
                        .child(t!(section).to_string()),
                );
            }
            body = body.child(
                h_flex()
                    .py_5()
                    .gap_6()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_base().child(t!(row_title).to_string()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(t!(detail).to_string()),
                            )
                            .when(*row == Row::Autostart, |this| {
                                let startup = cx.global::<AutostartStore>();
                                this.when_some(
                                    startup.status.and_then(|status| status.notice_key()),
                                    |this, key| {
                                        this.child(
                                            div()
                                                .text_sm()
                                                .text_color(muted)
                                                .child(t!(key).to_string()),
                                        )
                                    },
                                )
                                .when_some(
                                    startup.error.as_ref(),
                                    |this, error| {
                                        this.child(
                                            div().text_sm().text_color(danger).child(
                                                t!("settings.autostart_error", error = error)
                                                    .to_string(),
                                            ),
                                        )
                                    },
                                )
                            }),
                    )
                    .child(div().w_56().flex_none().child(self.control(*row, cx))),
            );
        }
        let show_translate = if query.is_empty() {
            self.page == Page::Translate
        } else {
            [
                "translate.title",
                "translate.settings_subtitle",
                "translate.endpoint",
                "translate.model",
                "translate.key_title",
            ]
            .iter()
            .any(|key| t!(*key).to_lowercase().contains(query))
                || "deepseek api key endpoint".contains(query)
        };
        if show_translate {
            // Unrelated settings pages must not trigger a Keychain read or authorization dialog.
            let translate = self.translate.get_or_insert_with(|| {
                cx.new(|cx| super::translate::TranslateSettingsView::new(window, cx))
            });
            body = body.child(translate.clone());
        }
        if rows.is_empty() && !show_translate {
            body = body.child(
                div()
                    .pt_10()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("settings.no_match").to_string()),
            );
        }

        // Native startup registration cannot be edited in settings.toml.
        let show_settings_file = show_translate || rows.iter().any(|row| *row != Row::Autostart);
        v_flex()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .px_8()
            .pt_5()
            .pb_8()
            .overflow_y_scroll()
            .when(show_settings_file, |this| {
                this.child(
                    h_flex()
                        .h_7()
                        .gap_4()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(mono_font)
                                .text_sm()
                                .text_color(muted)
                                .child(self.path_label.clone()),
                        )
                        .child(
                            Button::new("edit-file")
                                .outline()
                                .small()
                                .label(t!("settings.edit_file"))
                                .on_click(cx.listener(|this, _, _, cx| this.edit_file(cx))),
                        ),
                )
            })
            .child(
                v_flex()
                    .pt_5()
                    .gap_0p5()
                    .child(div().text_lg().font_medium().child(title.to_string()))
                    .when_some(subtitle, |this, subtitle| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(subtitle.to_string()),
                        )
                    }),
            )
            .child(body)
    }
}

impl Focusable for SettingsWindow {
    /// The page itself takes focus so Esc reaches it.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    /// Client-side title bar above the sidebar and the content.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close))
            .child(TitleBar::new())
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_sidebar(!query.is_empty(), window, cx))
                    .child(self.render_content(&query, window, cx)),
            )
    }
}
