//! What the engine tells the UI to show. Plain data: the UI renders it and keeps no state of its
//! own beyond the form fields the user is typing into.

use crate::session::LinkKind;
use crate::settings::Mode;
use crate::signal::SignalView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseView {
    Idle,
    Connecting,
    Active,
    Stopping,
}

/// Secondary line of the status row. The UI formats times in the user's time zone.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusDetail {
    ChooseDuration,
    None,
    /// "Until 17:23 · host"
    Until { expires_unix: f64, host: String },
    /// Free text from the server or the network layer; may be long.
    Message(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusView {
    pub icon: &'static str,
    /// `success`, `warning`, `error`, or empty.
    pub class: &'static str,
    pub title: String,
    pub detail: StatusDetail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryButton {
    Start,
    StartDisabled,
    Stop,
    StopDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkView {
    pub id: String,
    pub url: String,
    pub kind: LinkKind,
    /// "Your location only", "Group share (host)", "Group share (member)".
    pub subtitle: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeOffer {
    pub links: usize,
    pub expires_unix: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationView {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub phase: PhaseView,
    pub status: StatusView,
    pub signal: SignalView,
    pub primary: PrimaryButton,
    /// The "New Share" form is editable only while idle.
    pub form_sensitive: bool,
    pub mode: Mode,
    pub group_pin: Option<String>,
    pub links: Vec<LinkView>,
    pub can_add_link: bool,
    pub expires_unix: Option<f64>,
    pub resume_offer: Option<ResumeOffer>,
    pub notification: Option<NotificationView>,
}

/// A transient message. `copy_link` adds a "Copy Link" button that copies the primary link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub copy_link: bool,
}

impl Toast {
    pub fn text(title: impl Into<String>) -> Self {
        Self { title: title.into(), copy_link: false }
    }
}

/// An error or a decision the user has to read. `open_preferences` adds a button that opens
/// Preferences (used when the fix is a setting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogSpec {
    pub heading: String,
    pub body: String,
    pub open_preferences: bool,
}

impl DialogSpec {
    pub fn error(heading: impl Into<String>, body: impl Into<String>) -> Self {
        Self { heading: heading.into(), body: body.into(), open_preferences: false }
    }
}
