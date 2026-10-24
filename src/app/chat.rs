//! Transcript shell: sidebar rows, model bar, title bar, message rows.

use super::*;

impl KiiChat {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let sidebar_bg = theme.sidebar;
        let sidebar_border = theme.sidebar_border;
        let sidebar_fg = theme.sidebar_foreground;
        let radius = theme.radius;
        let accent = theme.sidebar_accent;

        let mut sidebar = v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(sidebar_border)
            .text_color(sidebar_fg);

        match self.page {
            Page::Chat => {
                sidebar = sidebar
                    .child(
                        div().p_2().child(
                            Button::new("new-session")
                                .label("新建对话")
                                .icon(IconName::Plus)
                                .small()
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                        ),
                    );
                // The search box earns its row once the list is long enough to
                // scan; below that it is chrome.
                if self.store.sessions.len() >= SESSION_SEARCH_MIN {
                    sidebar = sidebar.child(
                        div().px_2().pb_1().child(Input::new(&self.session_search)),
                    );
                }
                sidebar = sidebar.child(
                    div()
                        .id("session-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(v_flex().px_2().pb_2().gap(px(2.)).children(self.session_rows(cx))),
                );
                // Settings is a destination of the sidebar, not a title-bar
                // utility: it sits below the conversation list and becomes
                // 「返回对话」 on the settings page.
                sidebar = sidebar.child(
                    div()
                        .p_2()
                        .border_t_1()
                        .border_color(sidebar_border)
                        .child(
                            h_flex()
                                .id("sidebar-settings")
                                .role(gpui::Role::Button)
                                .aria_label("设置")
                                .gap_2()
                                .px_2()
                                .py_2()
                                .rounded(radius)
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.show_settings(window, cx);
                                }))
                                .child(Icon::new(IconName::Settings).small())
                                .child(div().child("设置")),
                        ),
                );
            }
            Page::Settings => {
                let accent_fg = theme.sidebar_accent_foreground;
                sidebar = sidebar
                    .child(
                        div().flex_1().min_h_0().overflow_y_scrollbar().child(
                            v_flex()
                                .gap(px(2.))
                                .p_2()
                                .children(self.section_rows(accent, accent_fg, cx)),
                        ),
                    )
                    .child(
                        div()
                            .p_2()
                            .border_t_1()
                            .border_color(sidebar_border)
                            .child(
                                h_flex()
                                    .id("sidebar-back")
                                    .role(gpui::Role::Button)
                                    .aria_label("返回对话")
                                    .gap_2()
                                    .px_2()
                                    .py_2()
                                    .rounded(radius)
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(accent))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.page = Page::Chat;
                                        cx.notify();
                                    }))
                                    .child(Icon::new(IconName::ArrowLeft).small())
                                    .child(div().child("返回对话")),
                            ),
                    );
            }
        }
        sidebar.into_any_element()
    }

    pub(super) fn session_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let accent = theme.sidebar_accent;
        let accent_fg = theme.sidebar_accent_foreground;
        let sessions = self.visible_sessions(cx);
        sessions
            .into_iter()
            .enumerate()
            .map(|(ix, (id, title))| {
                let active = self.current.as_deref() == Some(id.as_str());

                // Rename replaces the row in place: the list keeps its shape
                // instead of opening a dialog over the transcript.
                if self.renaming.as_deref() == Some(id.as_str()) {
                    let save = cx.listener(|this, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.submit_rename(cx);
                    });
                    let cancel = cx.listener(|this, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.cancel_rename(cx);
                    });
                    return h_flex()
                        .id(("rename-session", ix))
                        .gap_1()
                        .px_1()
                        .py_1()
                        .child(div().flex_1().min_w_0().child(Input::new(&self.rename_input)))
                        .child(
                            Button::new("rename-save")
                                .icon(IconName::Check)
                                .ghost()
                                .small()
                                .tooltip("保存名称")
                                .accessibility_label("保存名称")
                                .on_click(save),
                        )
                        .child(
                            Button::new("rename-cancel")
                                .icon(IconName::Close)
                                .ghost()
                                .small()
                                .tooltip("取消重命名")
                                .accessibility_label("取消重命名")
                                .on_click(cancel),
                        )
                        .into_any_element();
                }

                let select = cx.listener({
                    let id = id.clone();
                    move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.select_session(&id, cx);
                    }
                });
                let rename = cx.listener({
                    let id = id.clone();
                    move |this: &mut Self,
                          _: &ClickEvent,
                          window: &mut Window,
                          cx: &mut Context<Self>| {
                        // A row button would otherwise bubble to the row's
                        // select handler, which clears `renaming` immediately.
                        cx.stop_propagation();
                        this.begin_rename(&id, window, cx);
                    }
                });
                let remove = cx.listener({
                    let id = id.clone();
                    move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        cx.stop_propagation();
                        this.request_delete_session(&id, cx);
                    }
                });
                h_flex()
                    .id(SharedString::from(format!("session-{}", id)))
                    .group("session-row")
                    .role(gpui::Role::Button)
                    .aria_label(format!("会话：{title}"))
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .py_1()
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |this| this.bg(accent).text_color(accent_fg))
                    .on_click(select)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(px(1.))
                            .opacity(0.)
                            .group_hover("session-row", |this| this.opacity(1.))
                            .child(
                                Button::new(SharedString::from(format!("rename-{}", id)))
                                    .icon(Icon::empty().path(icons::SQUARE_PEN_PATH))
                                    .ghost()
                                    .small()
                                    .tooltip("重命名会话")
                                    .accessibility_label("重命名会话")
                                    .on_click(rename),
                            )
                            .child(
                                Button::new(SharedString::from(format!("delete-{}", id)))
                                    .icon(IconName::Delete)
                                    .ghost()
                                    .small()
                                    .tooltip("删除会话")
                                    .accessibility_label("删除会话")
                                    .on_click(remove),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// Sidebar rows while settings are open: one per settings section.
    pub(super) fn section_rows(&self, accent: Hsla, accent_fg: Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        Section::ALL
            .iter()
            .map(|(section, label, icon)| {
                let section = *section;
                let active = self.section == section;
                let select = cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                    this.section = section;
                    cx.notify();
                });
                h_flex()
                    .id(SharedString::from(format!("section-{}", section.title())))
                    .role(gpui::Role::Button)
                    .aria_label(format!("设置项：{label}"))
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |this| this.bg(accent).text_color(accent_fg))
                    .on_click(select)
                    .child(Icon::new(icon.clone()).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(*label),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_chat(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut pane = v_flex().flex_1().min_w_0().h_full().gap_2().p_3();

        if let Some(notice) = self.notice.clone() {
            pane = pane.child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.secondary_foreground)
                            .child(notice),
                    )
                    .child(
                        Button::new("dismiss-notice")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .tooltip("关闭提示")
                            .accessibility_label("关闭提示")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }

        let empty = self
            .current_session()
            .is_none_or(|session| session.messages.is_empty());
        let body = if empty {
            self.render_welcome(cx)
        } else {
            let state = self.transcript.clone();
            // The list is a custom element: without an explicit size it lays
            // out to zero and paints nothing.
            div()
                .size_full()
                .child(list(state, cx.processor(Self::render_row)).size_full())
                .into_any_element()
        };

        // The backdrop covers the transcript while the picker floats above the
        // composer; the popup is deferred, so it does not reflow anything.
        let backdrop = self.picker_open.then(|| {
            div()
                .id("picker-backdrop")
                .absolute()
                .inset_0()
                .on_click(cx.listener(|this, _, _, cx| this.close_picker(cx)))
                .into_any_element()
        });

        pane.child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(body)
                .when_some(backdrop, |this, backdrop| this.child(backdrop)),
        )
        .child(self.render_composer(cx))
        .into_any_element()
    }

    /// The model picker: a deferred, anchored popup above the composer's model
    /// pill. It lists every provider's models, so switching endpoint and model
    /// is one choice.
    pub(super) fn render_model_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let rows = self.picker_rows(cx);
        let current_provider = self.effective_provider_id();
        let current_model = self.effective_model();
        let result_count = rows.len();
        let row_elements: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let is_current = current_provider.as_deref() == Some(row.provider_id.as_str())
                    && current_model.as_deref() == Some(row.model_id.as_str());
                let select = cx.listener({
                    let provider_id = row.provider_id.clone();
                    let model_id = row.model_id.clone();
                    move |this: &mut Self,
                          _: &ClickEvent,
                          window: &mut Window,
                          cx: &mut Context<Self>| {
                        this.pick_model(&provider_id, &model_id, window, cx);
                    }
                });
                h_flex()
                    .id(("pick-model", ix))
                    .role(gpui::Role::Button)
                    .aria_label(format!("模型：{}（{}）", row.model_id, row.provider_name))
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .py(px(6.))
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(is_current, |this| {
                        this.bg(theme.accent).text_color(theme.accent_foreground)
                    })
                    .on_click(select)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(row.model_id.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if is_current {
                                theme.accent_foreground
                            } else {
                                muted
                            })
                            .child(row.provider_name.clone()),
                    )
                    .when(is_current, |this| {
                        this.child(Icon::new(IconName::Check).small())
                    })
                    .into_any_element()
            })
            .collect();

        let empty = row_elements.is_empty();
        let needs_setup = self
            .store
            .providers
            .iter()
            .all(|provider| provider.models.is_empty());
        let open_settings = cx.listener(|this, _, window, cx| {
            this.picker_open = false;
            this.show_settings(window, cx);
        });
        let close = cx.listener(|this, _, _, cx| this.close_picker(cx));

        let panel_body: AnyElement = if empty {
            v_flex()
                .gap_2()
                .p_1()
                .text_xs()
                .text_color(muted)
                .child(if self.store.providers.is_empty() {
                    "还没有供应商。"
                } else if needs_setup {
                    "供应商还没有模型，先去获取模型。"
                } else {
                    "没有匹配的模型。"
                })
                .when(needs_setup, |this| {
                    this.child(
                        Button::new("picker-open-settings")
                            .label("打开设置")
                            .small()
                            .on_click(open_settings),
                    )
                })
                .into_any_element()
        } else {
            v_flex()
                .gap(px(2.))
                .children(row_elements)
                .into_any_element()
        };

        let panel = v_flex()
            .id("model-picker-panel")
            .occlude()
            .w(px(360.))
            .max_w(relative(0.92))
            .gap_2()
            .p_3()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_sm().font_semibold().child("选择模型"))
                    .child(
                        Button::new("close-picker")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .tooltip("关闭")
                            .accessibility_label("关闭模型选择")
                            .on_click(close),
                    ),
            )
            .child(Input::new(&self.picker_search))
            .child(
                div()
                    .id("model-picker-list")
                    .max_h(px(280.))
                    .overflow_y_scrollbar()
                    .child(panel_body),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().text_xs().text_color(muted).child(if empty {
                            "选择后会同时切换该会话的供应商与模型。".to_string()
                        } else {
                            format!("{result_count} 个模型")
                        }),
                    )
                    .when(!empty, |this| {
                        this.child(
                            Button::new("pick-first")
                                .label("选择第一个")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pick_first_match(window, cx);
                                })),
                        )
                    }),
            );

        // The pill's bounds are in window space; anchoring the popup's
        // bottom-right corner there makes it grow upward from the composer,
        // over the transcript, with no layout shift.
        let anchor = gpui::point(
            self.model_pill_bounds.origin.x + self.model_pill_bounds.size.width,
            self.model_pill_bounds.origin.y,
        );
        gpui::deferred(
            gpui::anchored()
                .position(anchor)
                .anchor(gpui::Anchor::BottomRight)
                .snap_to_window_with_margin(px(12.))
                .child(panel),
        )
        .with_priority(gpui_base::POPUP_PRIORITY)
        .into_any_element()
    }

    /// The app's composer: a DSH-style card with the model pill and a round
    /// send / stop control on its bottom row.
    pub(super) fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let popup = self.picker_open.then(|| self.render_model_picker(cx));
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let running = self.stream.is_some();
        let has_draft = !self.composer.read(cx).value().trim().is_empty();
        let submit = cx.listener(|this, _, window, cx| this.submit_composer(window, cx));
        let stop = cx.listener(|this, _, _, cx| this.cancel_stream(cx));
        let current = self
            .effective_model()
            .unwrap_or_else(|| "选择模型".into());
        let provider = self
            .effective_provider_id()
            .and_then(|id| self.store.provider(&id))
            .map(|provider| provider.name.clone());

        let toggle = cx.listener(|this, _, window, cx| {
            if this.picker_open {
                this.close_picker(cx);
            } else {
                this.open_picker(window, cx);
            }
        });
        let trigger = Button::new("open-picker")
            .label(current)
            .icon(IconName::ChevronsUpDown)
            .secondary()
            .small()
            .rounded(px(10.))
            .accessibility_label("选择模型")
            .on_click(toggle);
        // A tooltip over the pill would paint on top of the popup while the
        // popup is open; it is only useful as a closed-state affordance.
        let trigger = if self.picker_open {
            trigger
        } else {
            trigger.tooltip(match provider {
                Some(provider) => format!("选择模型 · {provider}"),
                None => "选择模型".to_string(),
            })
        };

        // Record the pill's window-space bounds so the deferred popup can hang
        // above it without being an absolute child of this small row.
        let state = cx.entity();
        let pill = div()
            .on_prepaint(move |bounds, _, cx| {
                state.update(cx, |this, _| {
                    this.model_pill_bounds = bounds;
                });
            })
            .child(trigger);

        v_flex()
            .flex_none()
            .gap_2()
            .p_3()
            .rounded(px(16.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .child(
                Textarea::new(&self.composer)
                    .appearance(false)
                    .bordered(false)
                    .w_full()
                    .min_h(px(40.)),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().text_color(muted).child(if running {
                        "生成中，可随时停止"
                    } else {
                        "Enter 发送 · Shift+Enter 换行"
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(pill)
                            .child(if running {
                                Button::new("composer-stop")
                                    .icon(IconName::Pause)
                                    .primary()
                                    .rounded_full()
                                    .w(px(32.))
                                    .h(px(32.))
                                    .justify_center()
                                    .tooltip("停止生成")
                                    .accessibility_label("停止生成")
                                    .on_click(stop)
                                    .into_any_element()
                            } else {
                                Button::new("composer-send")
                                    .icon(IconName::ArrowUp)
                                    .primary()
                                    .rounded_full()
                                    .w(px(32.))
                                    .h(px(32.))
                                    .justify_center()
                                    .disabled(!has_draft)
                                    .tooltip("发送")
                                    .accessibility_label("发送")
                                    .on_click(submit)
                                    .into_any_element()
                            }),
                    ),
            )
            .children(popup)
            .into_any_element()
    }

    /// The window's own title bar: the title, the sidebar toggle, then the
    /// app's toggles and the system window controls.
    ///
    /// gpui-component's `TitleBar` cannot host controls, and neither can a
    /// drag region that sits in the same row: on Windows the platform reads a
    /// registered `WindowControlArea::Drag` for the whole row, so clicks on
    /// anything beside it become window drags. Only the title text carries the
    /// drag area; every control is a sibling of it.
    pub(super) fn render_title_bar(
        &self,
        window: &mut Window,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let collapsed = self.store.sidebar_collapsed;
        let in_settings = self.page == Page::Settings;
        let controls = window.window_controls();

        let mut left = h_flex()
            .flex_none()
            .gap_2()
            .pl_3()
            .child(
                div()
                    .id("title-drag")
                    .text_sm()
                    .font_semibold()
                    .window_control_area(WindowControlArea::Drag)
                    .child("KiiChat"),
            );
        if !in_settings {
            left = left.child(
                Button::new("toggle-sidebar")
                    .icon(if collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .ghost()
                    .small()
                    .tooltip(if collapsed { "展开侧边栏" } else { "折叠侧边栏" })
                    .accessibility_label("折叠或展开侧边栏")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.sidebar_collapsed = !this.store.sidebar_collapsed;
                        this.persist(cx);
                        cx.notify();
                    })),
            );
        }

        let mut bar = h_flex()
            .flex_shrink_0()
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            .items_center()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.title_bar_border)
            .text_color(theme.foreground)
            .child(left)
            .child(div().flex_1().min_w_0())
            .child(
                Button::new("toggle-theme")
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .ghost()
                    .small()
                    .tooltip(if dark { "切换到浅色" } else { "切换到深色" })
                    .accessibility_label("切换深浅色")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
            );

        // The sidebar owns the settings entry while it is open. The title-bar
        // slot keeps three jobs: a fallback settings button when the sidebar is
        // collapsed, and the back button on the settings page.
        if in_settings || collapsed {
            bar = bar.child(
                Button::new("toggle-page")
                    .icon(if in_settings {
                        IconName::ArrowLeft
                    } else {
                        IconName::Settings
                    })
                    .ghost()
                    .small()
                    .tooltip(if in_settings { "返回对话" } else { "设置" })
                    .accessibility_label(if in_settings { "返回对话" } else { "设置" })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.page == Page::Settings {
                            this.page = Page::Chat;
                            cx.notify();
                        } else {
                            this.show_settings(window, cx);
                        }
                    })),
            );
        }

        // macOS draws its own controls; everywhere else the platform hit-tests
        // the areas below (Windows) or we drive them by hand.
        if !cfg!(target_os = "macos") {
            bar = bar
                .when(controls.minimize, |this| {
                    this.child(control_button(
                        "window-minimize",
                        "最小化",
                        IconName::WindowMinimize,
                        WindowControlArea::Min,
                        theme.secondary_hover,
                        theme.secondary_active,
                        theme.foreground,
                        theme.foreground,
                        |window, _| window.minimize_window(),
                    ))
                })
                .when(controls.maximize, |this| {
                    this.child(control_button(
                        "window-maximize",
                        if window.is_maximized() { "还原" } else { "最大化" },
                        if window.is_maximized() {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        },
                        WindowControlArea::Max,
                        theme.secondary_hover,
                        theme.secondary_active,
                        theme.foreground,
                        theme.foreground,
                        |window, _| window.zoom_window(),
                    ))
                })
                .child(control_button(
                    "window-close",
                    "关闭",
                    IconName::WindowClose,
                    WindowControlArea::Close,
                    // Idle ink matches min/max so the X is never invisible;
                    // only hover turns the control red with a light glyph.
                    theme.danger,
                    theme.danger_active,
                    theme.foreground,
                    gpui::rgb(0xffffff).into(),
                    |window, _| window.remove_window(),
                ));
        }
        bar.into_any_element()
    }

    pub(super) fn render_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let has_provider = !self.store.providers.is_empty();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Orbs::new().diameter(px(48.)))
            .child(div().text_xl().font_semibold().child("开始新的对话"))
            .child(
                div()
                    .max_w(relative(0.7))
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .text_center()
                    .child(if has_provider {
                        "在下方输入消息，Enter 发送。点输入框右下角的模型名可以切换供应商与模型。"
                    } else {
                        "先点左侧栏底部的「设置」，到「模型」添加一个 OpenAI 兼容的接口，再回来对话。"
                    }),
            )
            .into_any_element()
    }

    /// One transcript row: a bubble plus the actions that apply to it.
    pub(super) fn render_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self
            .current_session()
            .and_then(|session| session.messages.get(index))
            .cloned()
        else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let stopped_ink = theme.muted_foreground;
        let is_user = message.role == Role::User;
        let failed = message.error.as_deref().filter(|error| !error.is_empty());
        let streaming = self
            .stream
            .as_ref()
            .is_some_and(|stream| stream.message_id == message.id);
        let stopped = message.stopped && !streaming;
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.message_id == message.id)
            .map(|editing| editing.editor.clone());

        let content_id = ElementId::from((SharedString::from(message.id.clone()), index));
        let body: AnyElement = match &editing {
            Some(editor) => v_flex()
                .gap_2()
                .child(Textarea::new(editor).w_full())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(("save-edit", index))
                                .label("保存并重发")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.submit_edit(cx))),
                        )
                        .child(
                            Button::new(("cancel-edit", index))
                                .label("取消")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                        ),
                )
                .into_any_element(),
            None => {
                let content = match failed {
                    Some(error) => StreamedContent::failed(message.content.clone(), error),
                    None if streaming => StreamedContent::running(message.content.clone()),
                    None => StreamedContent::done(message.content.clone()),
                };
                if is_user {
                    TextView::markdown(content_id, content.text())
                        .selectable(true)
                        .into_any_element()
                } else {
                    StreamingText::new(content_id, &content).into_any_element()
                }
            }
        };

        let bubble = v_flex()
            .id(ElementId::from((
                SharedString::from(message.id.clone()),
                index + 1_000_000,
            )))
            .min_w_0()
            .max_w(relative(0.82))
            // Room between the answer and the actions under it.
            .gap(px(12.))
            .px_3()
            .py_2()
            .rounded(theme.radius_lg)
            .when(is_user && editing.is_none(), |this| this.bg(theme.secondary))
            .when(editing.is_some(), |this| {
                this.border_1().border_color(theme.primary)
            })
            .when(failed.is_some(), |this| this.border_1().border_color(theme.danger))
            .when_some(
                {
                    let muted = theme.muted_foreground;
                    let muted_bg = theme.muted;
                    let radius = theme.radius;
                    self.render_thinking_strip(index, &message, muted, muted_bg, radius, cx)
                },
                |this, strip| this.child(strip),
            )
            .child(body)
            .when(stopped, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(stopped_ink)
                        .child("已停止生成"),
                )
            })
            .when(!streaming && editing.is_none(), |this| {
                this.child(self.render_actions(index, &message, failed.is_some(), window, cx))
            });

        v_flex()
            .id(ElementId::from((
                SharedString::from(message.id.clone()),
                index + 2_000_000,
            )))
            .w_full()
            .px_4()
            .py_2()
            .when(is_user, |this| this.items_end())
            .child(bubble)
            .into_any_element()
    }

    /// Icon-only actions for one message.
    ///
    /// A failed reply's retry grows a red label, so the way out of a failure
    /// sits where every other action lives.
    pub(super) fn render_actions(
        &self,
        index: usize,
        message: &Msg,
        failed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let is_user = message.role == Role::User;
        let _ = window;

        let copy = cx.listener({
            let text = message.content.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }
                this.notice = None;
                cx.notify();
            }
        });
        let branch = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                this.branch_from(&id, cx);
            }
        });
        let retry = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                this.retry(&id, cx);
            }
        });
        let edit = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>| {
                this.begin_edit(&id, window, cx);
            }
        });

        let mut actions = h_flex()
            .w_full()
            .gap(px(2.))
            // User bubbles hug the right edge, replies the left.
            .when(is_user, |this| this.justify_end())
            .child(
                Button::new(("copy", index))
                    .icon(IconName::Copy)
                    .ghost()
                    .small()
                    .tooltip("复制")
                    .accessibility_label("复制")
                    .on_click(copy),
            )
            .child(
                Button::new(("branch", index))
                    .icon(Icon::empty().path(icons::GIT_BRANCH_PATH))
                    .ghost()
                    .small()
                    .tooltip("以此消息为起点新建会话")
                    .accessibility_label("分支")
                    .on_click(branch),
            );

        if is_user {
            actions = actions.child(
                Button::new(("edit", index))
                    .icon(Icon::empty().path(icons::SQUARE_PEN_PATH))
                    .ghost()
                    .small()
                    .tooltip("编辑并重发")
                    .accessibility_label("编辑")
                    .on_click(edit),
            );
        } else {
            let retry_button = Button::new(("retry", index))
                .icon(IconName::RotateCw)
                .small()
                .tooltip("重新生成")
                .accessibility_label("重试")
                .on_click(retry);
            actions = actions.child(if failed {
                // Expanded and inked, without a fill: the failure earns the
                // extra attention, the icon earns the color.
                retry_button
                    .label("重试")
                    .text()
                    .text_color(theme.danger)
            } else {
                retry_button.ghost()
            });
        }
        actions.into_any_element()
    }

    /// Collapsible reasoning strip above the answer. Default open while
    /// streaming; after completion it stays collapsed until clicked.
    pub(super) fn render_thinking_strip(
        &self,
        index: usize,
        message: &Msg,
        muted: Hsla,
        muted_bg: Hsla,
        radius: Pixels,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let thinking = message.thinking.as_deref()?.trim();
        if thinking.is_empty() {
            return None;
        }
        let streaming = self
            .stream
            .as_ref()
            .is_some_and(|stream| stream.message_id == message.id);
        let open = streaming || self.thinking_open.contains(&message.id);
        let chars = thinking.chars().count();
        let message_id = message.id.clone();
        let toggle = cx.listener({
            let id = message_id.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                if !this.thinking_open.remove(&id) {
                    this.thinking_open.insert(id.clone());
                }
                let session_id = this.current.clone();
                if let Some(session_id) = session_id
                    && let Some(row) = this.row_of(&session_id, &id)
                {
                    this.transcript.remeasure_items(row..row + 1);
                }
                cx.notify();
            }
        });

        let header = h_flex()
            .id(("thinking-toggle", index))
            .gap_1()
            .px_1()
            .py(px(2.))
            .rounded(radius)
            .text_xs()
            .text_color(muted)
            .cursor_pointer()
            .on_click(toggle)
            .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).small())
            .child(if streaming {
                "思考中…".to_string()
            } else if open {
                "收起思考过程".to_string()
            } else {
                format!("思考过程 · {chars} 字")
            });

        let mut strip = v_flex().gap_1().mb_2().child(header);
        if open {
            strip = strip.child(
                div()
                    .id(("thinking-body", index))
                    .px_2()
                    .py_1()
                    .rounded(radius)
                    .bg(muted_bg)
                    .text_xs()
                    .text_color(muted)
                    .max_h(px(160.))
                    .overflow_y_scrollbar()
                    .child(thinking.to_string()),
            );
        }
        Some(strip.into_any_element())
    }
}
