//! HTTP for the Hauk calls, over libsoup 3 on the GLib main loop.
//!
//! One long-lived session is reused so keep-alive saves a TLS handshake per post. Proxies come from
//! the system (GProxyResolver) unless the user picked one, TLS from glib-networking and the system
//! certificate store. The weaker certificate policies apply to `.onion` hosts only.

use std::cell::RefCell;
use std::rc::Rc;

use async_channel::Sender;
use hauk::Version;
use hauk::response::Reply;
use jastrzab_core::engine::Event;
use jastrzab_core::settings::{ProxyKind, Settings, TlsPolicy, is_onion};
use jastrzab_core::uplink::HttpId;
use soup::prelude::*;

const USER_AGENT: &str = concat!("Jastrzab/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct Http {
    settings: Rc<RefCell<Settings>>,
    session: Rc<RefCell<Option<soup::Session>>>,
    events: Sender<Event>,
}

impl Http {
    pub fn new(settings: Rc<RefCell<Settings>>, events: Sender<Event>) -> Self {
        Self { settings, session: Rc::new(RefCell::new(None)), events }
    }

    /// Forget the session so the next request is made with the current network settings.
    pub fn reconfigure(&self) {
        self.session.borrow_mut().take();
    }

    pub fn send(&self, id: HttpId, base: String, request: hauk::Request) {
        let url = format!("{base}{}", request.endpoint());
        let body = request.body();
        let session = self.session();
        let tls = self.settings.borrow().tls;
        let events = self.events.clone();
        tracing::debug!(endpoint = request.endpoint(), id, "sending a request");
        glib::spawn_future_local(async move {
            let result = perform(&session, &url, body, tls).await;
            let _ = events.try_send(Event::HttpDone { id, result });
        });
    }

    fn session(&self) -> soup::Session {
        self.session.borrow_mut().get_or_insert_with(|| build_session(&self.settings.borrow())).clone()
    }
}

fn build_session(settings: &Settings) -> soup::Session {
    let session = soup::Session::new();
    session.set_user_agent(USER_AGENT);
    session.set_timeout(settings.timeout_secs);
    session.set_idle_timeout(60);
    session.set_accept_language_auto(true);
    let proxy = match settings.proxy {
        ProxyKind::System => None,
        ProxyKind::None => Some(gio::SimpleProxyResolver::new(None, &[] as &[&str])),
        ProxyKind::Http => Some(gio::SimpleProxyResolver::new(Some(&format!("http://{}:{}", settings.proxy_host, settings.proxy_port)), &[] as &[&str])),
        ProxyKind::Socks => Some(gio::SimpleProxyResolver::new(Some(&format!("socks5://{}:{}", settings.proxy_host, settings.proxy_port)), &[] as &[&str])),
    };
    if let Some(resolver) = proxy {
        session.set_proxy_resolver(Some(&resolver));
    }
    session
}

async fn perform(session: &soup::Session, url: &str, body: String, tls: TlsPolicy) -> Result<Reply, String> {
    let message = soup::Message::new("POST", url).map_err(|_| "The server address is not a valid web address.".to_string())?;
    // Never follow a redirect: it would turn the POST into a GET (and the PHP backend answers
    // 405), and the password in the body must not be sent anywhere the user did not type in. The
    // user sees "Received HTTP 301" and fixes the address (usually http:// that should be https://).
    message.set_flags(soup::MessageFlags::NO_REDIRECT);
    message.set_request_body_from_bytes(Some("application/x-www-form-urlencoded"), Some(&glib::Bytes::from_owned(body.into_bytes())));
    if tls != TlsPolicy::Always && is_onion(url) {
        message.connect_accept_certificate(move |_, _, errors| accept_onion_certificate(tls, errors));
    }
    let bytes = session.send_and_read_future(&message, glib::Priority::DEFAULT).await.map_err(|err| err.message().to_string())?;
    let version = message.response_headers().and_then(|headers| headers.one(hauk::VERSION_HEADER)).and_then(|v| Version::parse(&v));
    let status = u32::try_from(glib::translate::IntoGlib::into_glib(message.status())).unwrap_or(0) as u16;
    Ok(Reply { status, version, body: String::from_utf8_lossy(&bytes).into_owned() })
}

/// What a `.onion` certificate may get away with. An onion service's address is its key, so it has
/// no CA-issued certificate: `OnionAnyChain` tolerates a chain that does not lead to a trusted
/// root but still checks the name, `OnionAnyCertificate` accepts anything.
fn accept_onion_certificate(policy: TlsPolicy, errors: gio::TlsCertificateFlags) -> bool {
    match policy {
        TlsPolicy::Always => false,
        TlsPolicy::OnionAnyCertificate => true,
        TlsPolicy::OnionAnyChain => (errors - gio::TlsCertificateFlags::UNKNOWN_CA).is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onion_certificate_policies() {
        let unknown_ca = gio::TlsCertificateFlags::UNKNOWN_CA;
        let bad_name = gio::TlsCertificateFlags::BAD_IDENTITY | gio::TlsCertificateFlags::UNKNOWN_CA;
        assert!(!accept_onion_certificate(TlsPolicy::Always, unknown_ca));
        assert!(accept_onion_certificate(TlsPolicy::OnionAnyChain, unknown_ca));
        assert!(!accept_onion_certificate(TlsPolicy::OnionAnyChain, bad_name));
        assert!(accept_onion_certificate(TlsPolicy::OnionAnyCertificate, bad_name));
    }
}
