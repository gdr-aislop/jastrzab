//! The one window. Its structure is design §4.1:
//!
//! ```text
//! AdwApplicationWindow          default size 360x648, resizable, never non-resizable
//! └─ GtkBox (vertical)
//!    ├─ AdwHeaderBar            title, Main Menu
//!    └─ AdwToastOverlay
//!       └─ GtkScrolledWindow    vertical only
//!          └─ AdwClamp          max 480, tightening 360
//!             └─ GtkBox        resume offer, status, GPS signal, New Share, button, group PIN, links
//! ```
//!
//! The page scrolls vertically and never horizontally, so no amount of content can raise the
//! window's minimum size above the phone's screen.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::tr;
use jastrzab_core::view::{DialogSpec, PhaseView, PrimaryButton, Toast, View};

use super::group::GroupSection;
use super::links::LinksSection;
use super::share_form::ShareForm;
use super::signal_row::SignalSection;
use super::status::{ResumeSection, StatusSection};
use super::{dialogs, preferences, widgets::*};
use crate::controller::Controller;

/// What Phosh leaves an app in portrait: 360×648 (design rule 13).
const DEFAULT_WIDTH: i32 = 360;
const DEFAULT_HEIGHT: i32 = 648;

#[derive(Clone)]
pub struct MainWindow {
    inner: Rc<Inner>,
}

struct Inner {
    window: adw::ApplicationWindow,
    controller: Controller,
    toasts: adw::ToastOverlay,
    resume: ResumeSection,
    status: StatusSection,
    signal: SignalSection,
    form: Rc<ShareForm>,
    primary: gtk::Button,
    primary_mode: Cell<PrimaryButton>,
    group: GroupSection,
    links: LinksSection,
    phase: Cell<PhaseView>,
    ticker: RefCell<Option<glib::SourceId>>,
    allow_close: Cell<bool>,
}

impl MainWindow {
    pub fn new(controller: &Controller) -> Self {
        let settings = controller.settings();
        let window = adw::ApplicationWindow::builder()
            .application(&controller.app())
            .title(tr("Jastrzab"))
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .build();

        let form = Rc::new(ShareForm::new(&settings, &window));
        let primary = gtk::Button::builder().label(tr("Start Sharing")).halign(gtk::Align::Center).build();
        primary.add_css_class("pill");
        primary.add_css_class("suggested-action");

        let adoptable = {
            let form = form.clone();
            move || form.values().adoptable
        };
        let inner = Rc::new(Inner {
            window: window.clone(),
            controller: controller.clone(),
            toasts: adw::ToastOverlay::new(),
            resume: ResumeSection::new(controller),
            status: StatusSection::new(),
            signal: SignalSection::new(controller),
            form,
            primary,
            primary_mode: Cell::new(PrimaryButton::Start),
            group: GroupSection::new(controller, &window),
            links: LinksSection::new(controller, adoptable),
            phase: Cell::new(PhaseView::Idle),
            ticker: RefCell::new(None),
            allow_close: Cell::new(false),
        });
        let this = MainWindow { inner };
        this.build();
        this
    }

    fn build(&self) {
        let inner = &self.inner;

        // All page content lives in a vertical-only scroller, so that no amount of content can
        // raise the window's minimum size.
        let page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).margin_top(18).margin_bottom(18).margin_start(12).margin_end(12).build();
        page.append(inner.resume.widget());
        page.append(inner.status.widget());
        page.append(inner.signal.widget());
        page.append(inner.form.widget());
        page.append(&inner.primary);
        page.append(inner.group.widget());
        page.append(inner.links.widget());
        let clamp = adw::Clamp::builder().maximum_size(480).tightening_threshold(360).child(&page).build();
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
        inner.toasts.set_child(Some(&scroller));

        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        root.append(&self.header());
        root.append(&inner.toasts);
        inner.window.set_content(Some(&root));

        self.add_actions();
        self.connect_signals();
    }

    fn header(&self) -> adw::HeaderBar {
        let menu = gio::Menu::new();
        menu.append(Some(&tr("_Preferences")), Some("win.preferences"));
        menu.append(Some(&tr("_About Jastrzab")), Some("win.about"));
        let button = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).tooltip_text(tr("Main Menu")).build();
        set_accessible_label(&button, &tr("Main Menu"));
        let header = adw::HeaderBar::new();
        header.pack_end(&button);
        header
    }

    fn add_actions(&self) {
        let inner = &self.inner;
        let weak = Rc::downgrade(&self.inner);
        let action = |name: &str, f: Box<dyn Fn(&Rc<Inner>)>| {
            let action = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            action.connect_activate(move |_, _| {
                if let Some(inner) = weak.upgrade() {
                    f(&inner);
                }
            });
            inner.window.add_action(&action);
        };
        action("preferences", Box::new(|inner| preferences::show(&inner.window, &inner.controller)));
        action("about", Box::new(|inner| dialogs::about(&inner.window)));
        action("copy-link", Box::new(|inner| inner.controller.send(Event::CopyLink(None))));
        inner.controller.app().set_accels_for_action("win.preferences", &["<Control>comma"]);
    }

    fn connect_signals(&self) {
        let inner = &self.inner;

        let weak = Rc::downgrade(&self.inner);
        inner.primary.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            match inner.primary_mode.get() {
                PrimaryButton::Start => inner.controller.start(inner.form.values()),
                PrimaryButton::Stop => {
                    if inner.controller.settings().confirm_stop {
                        dialogs::confirm_stop(&inner.window, &inner.controller);
                    } else {
                        inner.controller.send(Event::Stop);
                    }
                }
                PrimaryButton::StartDisabled | PrimaryButton::StopDisabled => {}
            }
        });

        // "Visible" means in front: the signal check and the countdown stop when it is not.
        let weak = Rc::downgrade(&self.inner);
        inner.window.connect_is_active_notify(move |window| {
            let Some(inner) = weak.upgrade() else { return };
            inner.controller.send(Event::WindowVisible(window.is_active()));
            MainWindow { inner }.update_ticker();
        });

        // Closing while sharing asks once: keep going in the background, or stop.
        let weak = Rc::downgrade(&self.inner);
        inner.window.connect_close_request(move |_| {
            let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
            if inner.allow_close.get() || inner.phase.get() != PhaseView::Active {
                return glib::Propagation::Proceed;
            }
            let weak = Rc::downgrade(&inner);
            dialogs::keep_sharing(&inner.window, move |answer| {
                let Some(inner) = weak.upgrade() else { return };
                match answer {
                    Some(keep) => {
                        if !keep {
                            inner.controller.send(Event::Stop);
                        }
                        inner.allow_close.set(true);
                        inner.window.close();
                    }
                    None => {}
                }
            });
            glib::Propagation::Stop
        });

        let weak = Rc::downgrade(&self.inner);
        inner.window.connect_destroy(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            MainWindow { inner: inner.clone() }.stop_ticker();
            inner.controller.window_closed();
        });
    }

    pub fn present(&self) {
        self.inner.window.present();
    }

    pub fn render(&self, view: &View) {
        let inner = &self.inner;
        inner.phase.set(view.phase);
        inner.resume.render(view);
        inner.status.render(view);
        inner.signal.render(view);
        inner.form.render(view);
        inner.group.render(view);
        inner.links.render(view);

        let (label, add, remove, sensitive) = match view.primary {
            PrimaryButton::Start => (tr("Start Sharing"), "suggested-action", "destructive-action", true),
            PrimaryButton::StartDisabled => (tr("Start Sharing"), "suggested-action", "destructive-action", false),
            PrimaryButton::Stop => (tr("Stop Sharing"), "destructive-action", "suggested-action", true),
            PrimaryButton::StopDisabled => (tr("Stop Sharing"), "destructive-action", "suggested-action", false),
        };
        inner.primary_mode.set(view.primary);
        if inner.primary.label().as_deref() != Some(label.as_str()) {
            inner.primary.set_label(&label);
        }
        set_style_class(&inner.primary, &[add, remove], add);
        set_sensitive(&inner.primary, sensitive);
        self.update_ticker();
    }

    pub fn toast(&self, toast: &Toast) {
        let item = adw::Toast::new(&toast.title);
        if toast.copy_link {
            item.set_button_label(Some(&tr("Copy Link")));
            item.set_action_name(Some("win.copy-link"));
        }
        self.inner.toasts.add_toast(item);
    }

    pub fn dialog(&self, spec: &DialogSpec) {
        dialogs::error(&self.inner.window, spec, &self.inner.controller);
    }

    pub fn open_preferences(&self) {
        preferences::show(&self.inner.window, &self.inner.controller);
    }

    /// The countdown ticks once a second, but only while a share is running and the window is in
    /// front. Nothing else in the app ticks.
    fn update_ticker(&self) {
        let inner = &self.inner;
        let want = inner.window.is_active() && inner.status.is_counting();
        let running = inner.ticker.borrow().is_some();
        match (want, running) {
            (true, false) => {
                inner.status.tick();
                let weak = Rc::downgrade(&self.inner);
                let id = glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
                    Some(inner) => {
                        inner.status.tick();
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });
                *inner.ticker.borrow_mut() = Some(id);
            }
            (false, true) => self.stop_ticker(),
            _ => {}
        }
    }

    fn stop_ticker(&self) {
        if let Some(id) = self.inner.ticker.borrow_mut().take()
            && let Some(source) = glib::MainContext::default().find_source_by_id(&id)
        {
            source.destroy();
        }
    }
}
