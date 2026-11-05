// SPDX-License-Identifier: AGPL-3.0-only
//! The optional update check (ADR 0014): its settings for the UI and the daily check from the
//! core's hourly tick (`updates` does the request).

use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    messages::now_ms,
    store::keys,
    updates,
};

/// How often GitHub is asked at most.
const INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

impl App {
    pub(crate) fn update_settings_json(&self) -> Result<Value, ApiError> {
        let enabled = match self.store.setting(keys::UPDATE_CHECK)?.as_deref() {
            Some("1") => Value::Bool(true),
            Some(_) => Value::Bool(false),
            None => Value::Null,
        };
        let checked_at = self
            .store
            .setting(keys::UPDATE_CHECKED_AT)?
            .and_then(|at| at.parse::<i64>().ok());
        let latest = self
            .store
            .setting(keys::UPDATE_LATEST)?
            .and_then(|json| serde_json::from_str::<Value>(&json).ok())
            .filter(|latest| {
                latest["version"]
                    .as_str()
                    .is_some_and(|version| updates::is_newer(crate::VERSION, version))
            });
        Ok(json!({
            "available": self.update_check_built,
            "enabled": enabled,
            "checkedAt": checked_at,
            "current": crate::VERSION,
            "latest": latest,
            "checking": self.update_check_running.get(),
        }))
    }

    /// Start a check for a newer release if the user agreed and the last attempt is a day old
    /// (or `manual`, "Check for updates now"). The request runs as a task of its own, so the
    /// core goes on with everything else meanwhile (deep audit P01); its result arrives as
    /// events: `updateChecked {ok, manual}` always, `updateAvailable` once per new version.
    /// Returns whether a check was started. A manual one asked for during a check adopts it: the
    /// user is waiting for its answer now, whichever started it.
    pub(crate) fn start_update_check(&mut self, manual: bool) -> bool {
        if manual && self.update_check_running.get() {
            self.update_check_manual.set(true);
        }
        if !self.begin_update_check(manual) {
            return false;
        }
        let store = std::rc::Rc::clone(&self.store);
        let events = self.media_events.clone();
        let running = std::rc::Rc::clone(&self.update_check_running);
        let asked = std::rc::Rc::clone(&self.update_check_manual);
        running.set(true);
        asked.set(manual);
        tokio::task::spawn_local(async move {
            let result = updates::latest().await;
            running.set(false);
            for event in finish_update_check(&store, result, asked.take()) {
                let _ = events.send(crate::providers::CoreEvent::Call(event));
            }
        });
        true
    }

    /// Whether a check may start now; if so, the attempt is recorded first, so failures count
    /// toward the daily limit too (deep audit R01).
    pub(crate) fn begin_update_check(&self, manual: bool) -> bool {
        if !self.update_check_built
            || self.update_check_running.get()
            || self
                .store
                .setting(keys::UPDATE_CHECK)
                .ok()
                .flatten()
                .as_deref()
                != Some("1")
        {
            return false;
        }
        let attempted_at = self
            .store
            .setting(keys::UPDATE_ATTEMPTED_AT)
            .ok()
            .flatten()
            .and_then(|at| at.parse::<i64>().ok())
            .unwrap_or(0);
        let now = now_ms();
        // A clock set back doesn't stop the checks for good.
        if !manual && (attempted_at..attempted_at + INTERVAL_MS).contains(&now) {
            return false;
        }
        let _ = self
            .store
            .set_setting(keys::UPDATE_ATTEMPTED_AT, &now.to_string());
        true
    }
}

/// What a finished check tells the UI, in order. A failure changes no setting, so the event is
/// the only sign of it; `manual` tells the check the user asked for from the daily one.
fn finish_update_check(
    store: &crate::store::Store,
    result: Result<Option<updates::Release>, String>,
    manual: bool,
) -> Vec<Value> {
    let ok = result.is_ok();
    let available = match result {
        Ok(release) => record_release(store, release),
        Err(error) => {
            tracing::info!(%error, "update check failed");
            None
        }
    };
    available
        .into_iter()
        .chain(std::iter::once(
            json!({ "event": "updateChecked", "ok": ok, "manual": manual }),
        ))
        .collect()
}

/// A successful answer: remembered (`checkedAt`, the newest release); the `updateAvailable`
/// event for a newer version not notified before.
fn record_release(store: &crate::store::Store, release: Option<updates::Release>) -> Option<Value> {
    let _ = store.set_setting(keys::UPDATE_CHECKED_AT, &now_ms().to_string());
    // No release (yet, or any more): nothing to offer.
    let Some(release) = release else {
        let _ = store.delete_setting(keys::UPDATE_LATEST);
        return None;
    };
    let latest = json!({ "version": release.version, "url": release.url });
    let _ = store.set_setting(keys::UPDATE_LATEST, &latest.to_string());
    if !updates::is_newer(crate::VERSION, &release.version) {
        return None;
    }
    if store
        .setting(keys::UPDATE_NOTIFIED)
        .ok()
        .flatten()
        .as_deref()
        == Some(release.version.as_str())
    {
        return None;
    }
    let _ = store.set_setting(keys::UPDATE_NOTIFIED, &release.version);
    tracing::info!(version = release.version, "newer release available");
    Some(json!({
        "event": "updateAvailable",
        "version": release.version,
        "url": release.url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::StartConfig;

    fn new_app(update_check: bool) -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "test".into(),
                language: "en".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "test".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check,
            },
            &[4; 32],
        )
        .unwrap();
        (dir, app)
    }

    /// The check the user asked for says whether it worked, or a failure would leave no trace.
    #[tokio::test(flavor = "current_thread")]
    async fn a_finished_check_says_how_it_went() {
        let (_dir, app) = new_app(true);
        let failed = finish_update_check(&app.store, Err("no route".into()), true);
        assert_eq!(
            failed,
            [json!({ "event": "updateChecked", "ok": false, "manual": true })]
        );
        assert!(
            app.update_settings_json().unwrap()["checkedAt"].is_null(),
            "a failure doesn't count as checked"
        );
        let daily = finish_update_check(&app.store, Err("no route".into()), false);
        assert_eq!(daily[0]["manual"], false);

        let none_yet = finish_update_check(&app.store, Ok(None), true);
        assert_eq!(
            none_yet,
            [json!({ "event": "updateChecked", "ok": true, "manual": true })]
        );
        assert!(app.update_settings_json().unwrap()["checkedAt"].is_i64());

        let newer = updates::Release {
            version: "999.0.0".into(),
            url: "https://example.org/release".into(),
        };
        let found = finish_update_check(&app.store, Ok(Some(newer)), false);
        assert_eq!(found[0]["event"], "updateAvailable");
        assert_eq!(found[1]["event"], "updateChecked");
        assert_eq!(found[1]["ok"], true);
    }

    /// "Check now" during the daily check starts nothing, but its failure is the user's to hear.
    #[tokio::test(flavor = "current_thread")]
    async fn check_now_adopts_a_running_check() {
        let (_dir, mut app) = new_app(true);
        app.store.set_setting(keys::UPDATE_CHECK, "1").unwrap();
        app.update_check_running.set(true);
        assert!(!app.start_update_check(false), "one at a time");
        assert!(!app.update_check_manual.get(), "the daily tick asks nobody");
        assert!(!app.start_update_check(true), "one at a time");
        assert!(app.update_check_manual.get());
    }

    /// Nothing is asked before the user agreed, nor in a build without the check; a stored
    /// release only counts while it is newer than this version.
    #[tokio::test(flavor = "current_thread")]
    async fn checks_only_when_agreed() {
        let (_dir, app) = new_app(true);
        let settings = app.update_settings_json().unwrap();
        assert_eq!(settings["available"], true);
        assert_eq!(settings["enabled"], Value::Null, "not asked yet");
        assert!(!app.begin_update_check(true), "not asked yet: no request");
        app.store.set_setting(keys::UPDATE_CHECK, "0").unwrap();
        assert!(!app.begin_update_check(true), "declined: no request");
        assert!(
            app.store
                .setting(keys::UPDATE_ATTEMPTED_AT)
                .unwrap()
                .is_none()
        );
        // Agreed: one attempt, recorded before the request, then none for a day, whether it
        // succeeded or not; "Check now" may always.
        app.store.set_setting(keys::UPDATE_CHECK, "1").unwrap();
        assert!(app.begin_update_check(false));
        assert!(
            app.store
                .setting(keys::UPDATE_ATTEMPTED_AT)
                .unwrap()
                .is_some()
        );
        assert!(!app.begin_update_check(false), "the next tick waits a day");
        assert!(app.begin_update_check(true), "check now");
        app.store.set_setting(keys::UPDATE_CHECK, "0").unwrap();

        app.store
            .set_setting(keys::UPDATE_LATEST, r#"{"version":"999.0.0","url":"u"}"#)
            .unwrap();
        assert_eq!(
            app.update_settings_json().unwrap()["latest"]["version"],
            "999.0.0"
        );
        app.store
            .set_setting(keys::UPDATE_LATEST, r#"{"version":"0.0.1","url":"u"}"#)
            .unwrap();
        assert_eq!(app.update_settings_json().unwrap()["latest"], Value::Null);

        let (_dir, mut store_build) = new_app(false);
        store_build
            .store
            .set_setting(keys::UPDATE_CHECK, "1")
            .unwrap();
        assert!(!store_build.begin_update_check(true));
        assert_eq!(
            store_build.update_settings_json().unwrap()["available"],
            false
        );
        let reply = store_build
            .handle(r#"{"id":1,"cmd":"setUpdateCheck","enabled":true}"#)
            .await;
        assert!(reply.contains("\"ok\":false"), "{reply}");
    }
}
