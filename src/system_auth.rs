//! "Confirm it's you": asks for your login password (or fingerprint) before a saved
//! password is filled, copied or deleted.
//!
//! Stales never sees what you type. It asks polkit, the system service that also
//! guards things like mounting disks, and polkit shows its own dialog (from your
//! desktop, e.g. Noctalia, GNOME or KDE), checks the answer with PAM and only tells
//! Stales yes or no. Fingerprints work wherever your system's polkit login accepts
//! them (pam_fprintd in /etc/pam.d/polkit-1).
//!
//! It needs the small policy file `dev.stales.Stales.policy` installed in
//! /usr/share/polkit-1/actions/ (install.sh and the packages do this).

use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gtk::gio;
use gtk::glib::{self, prelude::ToVariant};

/// The polkit action from data/dev.stales.Stales.policy.
pub const ACTION: &str = "dev.stales.Stales.use-password";
/// Where the policy file has to be for polkit to know the action.
pub const POLICY_PATH: &str = "/usr/share/polkit-1/actions/dev.stales.Stales.policy";

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// You confirmed (or did so a few minutes ago).
    Allowed,
    /// Wrong password, cancelled, or no dialog could be shown.
    Refused,
    /// The policy file isn't installed.
    NotInstalled,
    /// polkit isn't reachable.
    Failed(String),
}

thread_local! {
    /// When you last confirmed. Cleared by "Lock now".
    static CONFIRMED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// A dialog is open right now (don't stack a second one).
    static ASKING: Cell<bool> = const { Cell::new(false) };
}

/// Forget the last confirmation, so the next use asks again.
pub fn forget() {
    CONFIRMED_AT.with(|c| c.set(None));
}

/// Is the policy file installed? (Only a hint for the settings page; the real test
/// is what polkit answers.)
pub fn installed() -> bool {
    std::path::Path::new(POLICY_PATH).exists()
}

/// Ask polkit, unless you confirmed less than `grace` ago.
pub async fn confirm(grace: Duration) -> Outcome {
    let recent = CONFIRMED_AT.with(|c| c.get()).is_some_and(|at| at.elapsed() < grace);
    if recent {
        return Outcome::Allowed;
    }
    if ASKING.with(|a| a.replace(true)) {
        return Outcome::Refused;
    }
    let outcome = ask_polkit().await;
    ASKING.with(|a| a.set(false));
    if outcome == Outcome::Allowed {
        CONFIRMED_AT.with(|c| c.set(Some(Instant::now())));
    }
    outcome
}

async fn ask_polkit() -> Outcome {
    let bus = match gio::bus_get_future(gio::BusType::System).await {
        Ok(bus) => bus,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    // Identify Stales by its connection on the system bus: polkit looks up the process
    // and its login session itself, so nothing here can be faked by a web page.
    let Some(name) = bus.unique_name() else { return Outcome::Failed("no bus name".into()) };
    let subject_details: HashMap<String, glib::Variant> = HashMap::from([("name".into(), name.to_variant())]);
    let subject = ("system-bus-name", subject_details);
    let details: HashMap<String, String> = HashMap::new();
    const ALLOW_USER_INTERACTION: u32 = 1;
    let params = (subject, ACTION, details, ALLOW_USER_INTERACTION, "").to_variant();

    let reply = bus
        .call_future(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            "org.freedesktop.PolicyKit1.Authority",
            "CheckAuthorization",
            Some(&params),
            Some(glib::VariantTy::new("((bba{ss}))").expect("valid type")),
            gio::DBusCallFlags::NONE,
            // You may take a while to type; polkit itself gives up after a few minutes.
            i32::MAX,
        )
        .await;
    match reply {
        Ok(value) => {
            if authorized(&value) {
                Outcome::Allowed
            } else {
                Outcome::Refused
            }
        }
        Err(e) if e.to_string().contains("not registered") => Outcome::NotInstalled,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// polkit's answer is `((is_authorized, is_challenge, details))`. Anything else
/// counts as no.
fn authorized(reply: &glib::Variant) -> bool {
    reply.try_child_value(0).and_then(|r| r.try_child_value(0)).and_then(|b| b.get::<bool>()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_polkits_signature() {
        let subject_details: HashMap<String, glib::Variant> = HashMap::from([("name".into(), ":1.5".to_variant())]);
        let details: HashMap<String, String> = HashMap::new();
        let params = (("system-bus-name", subject_details), ACTION, details, 1u32, "").to_variant();
        assert_eq!(params.type_().as_str(), "((sa{sv})sa{ss}us)");
    }

    #[test]
    fn reads_polkits_answer() {
        let details: HashMap<String, String> = HashMap::from([("polkit.result".into(), "auth_self".into())]);
        let no = ((false, true, details.clone()),).to_variant();
        let yes = ((true, false, details),).to_variant();
        assert!(!authorized(&no));
        assert!(authorized(&yes));
        assert!(!authorized(&().to_variant()));
        assert!(!authorized(&("true",).to_variant()));
    }

    #[test]
    fn policy_file_names_the_action() {
        let policy = include_str!("../data/dev.stales.Stales.policy");
        assert!(policy.contains(&format!("<action id=\"{ACTION}\">")));
        assert!(policy.contains("<allow_active>auth_self</allow_active>"));
        assert!(policy.contains("<allow_any>no</allow_any>"));
    }
}
