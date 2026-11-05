// SPDX-License-Identifier: AGPL-3.0-only
//! Work that waits for the network or for a slow file beside the core loop instead of inside it.
//!
//! All of the core's commands, connection events and replies go through one loop on one thread,
//! so whatever it waits for, everything waits for: the first reply at start, notifications, a
//! ringing call, hanging up. A long command therefore runs as a local task on [`Shared`], the part
//! of the core tasks may keep hold of, and comes back to the loop with its result: the loop then
//! finishes it on the [`App`] and sends the reply.
//!
//! [`Lane::InTurn`] work runs one at a time, in the order it was asked for, so that what depends
//! on what came before (a backup after turning Safe on, a confirmation after a scan) still sees
//! it, and rounds of maintenance don't overlap with any of it. Commands from anyone but the UI
//! (Threema Web, tests, what is queued behind a quit) run in place: they need the answer there.

use std::{future::Future, rc::Rc};

use serde_json::Value;
use tokio::{
    sync::{Semaphore, mpsc},
    task::AbortHandle,
};

use crate::{
    app::{ApiError, App, Shared},
    app_multidevice::SyncChange,
};

/// Whether work waits for the work before it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    InTurn,
    /// Nothing depends on what else is running, and someone is waiting: a call, a lookup.
    Free,
}

/// A reply the loop is to send.
pub(crate) struct Reply {
    pub(crate) request: u64,
    pub(crate) change: Option<SyncChange>,
    pub(crate) outcome: Result<Value, ApiError>,
}

type Finish = Box<dyn FnOnce(&mut App) -> Option<Reply>>;

/// Some detached work is through: the loop finishes it.
pub(crate) struct Finished {
    round: u64,
    finish: Finish,
}

struct Running {
    task: AbortHandle,
    replies: bool,
}

/// The work beside the loop.
pub(crate) struct DetachedWork {
    done: mpsc::UnboundedSender<Finished>,
    done_rx: Option<mpsc::UnboundedReceiver<Finished>>,
    turn: Rc<Semaphore>,
    running: Vec<Running>,
    /// Counts up when all of it is dropped: what had finished by then counts no more.
    round: u64,
    /// Replies still to come.
    pub(crate) replies_due: usize,
    pub(crate) maintenance_running: bool,
    /// Work nobody waits for an answer to: a wake-up holds the device up for it.
    pub(crate) quiet_running: usize,
}

impl Default for DetachedWork {
    fn default() -> Self {
        let (done, done_rx) = mpsc::unbounded_channel();
        Self {
            done,
            done_rx: Some(done_rx),
            turn: Rc::new(Semaphore::new(1)),
            running: Vec::new(),
            round: 0,
            replies_due: 0,
            maintenance_running: false,
            quiet_running: 0,
        }
    }
}

impl App {
    pub(crate) fn take_detached_work(&mut self) -> Option<mpsc::UnboundedReceiver<Finished>> {
        self.detached.done_rx.take()
    }

    fn spawn_detached<T: 'static, Fut: Future<Output = T> + 'static>(
        &mut self,
        lane: Lane,
        replies: bool,
        work: impl FnOnce(Rc<Shared>) -> Fut + 'static,
        finish: impl FnOnce(&mut App, T) -> Option<Reply> + 'static,
    ) {
        let shared = Rc::clone(&self.shared);
        let done = self.detached.done.clone();
        let round = self.detached.round;
        let turn = (lane == Lane::InTurn).then(|| Rc::clone(&self.detached.turn));
        let task = tokio::task::spawn_local(async move {
            let permit = match &turn {
                Some(turn) => turn.acquire().await.ok(),
                None => None,
            };
            let result = work(shared).await;
            drop(permit);
            let finish: Finish = Box::new(move |app| finish(app, result));
            let _ = done.send(Finished { round, finish });
        });
        self.detached
            .running
            .retain(|work| !work.task.is_finished());
        self.detached.running.push(Running {
            task: task.abort_handle(),
            replies,
        });
    }

    /// Runs a command's long part: beside the loop if the UI asked and it can reply later, else
    /// in place. `finish` then does what takes the rest of the core, between other commands.
    pub(crate) async fn detached_then<T: 'static, Fut: Future<Output = T> + 'static>(
        &mut self,
        lane: Lane,
        work: impl FnOnce(Rc<Shared>) -> Fut + 'static,
        finish: impl FnOnce(&mut App, T) -> Result<Value, ApiError> + 'static,
    ) -> Result<Value, ApiError> {
        if !self.in_background.get() {
            let result = Box::pin(work(Rc::clone(&self.shared))).await;
            return finish(self, result);
        }
        self.reply_later = true;
        self.detached.replies_due += 1;
        let request = self.current_request;
        let change = self.current_change.take();
        self.spawn_detached(lane, true, work, move |app, result| {
            app.detached.replies_due = app.detached.replies_due.saturating_sub(1);
            Some(Reply {
                request,
                change,
                outcome: finish(app, result),
            })
        });
        // Nobody reads this: the reply comes with the task's end.
        Ok(Value::Null)
    }

    /// [`Self::detached_then`] for a command with nothing left to do but to reply.
    pub(crate) async fn detached<Fut: Future<Output = Result<Value, ApiError>> + 'static>(
        &mut self,
        lane: Lane,
        work: impl FnOnce(Rc<Shared>) -> Fut + 'static,
    ) -> Result<Value, ApiError> {
        self.detached_then(lane, work, |_, outcome| outcome).await
    }

    /// Runs what nobody waits for an answer to (maintenance, publishing the feature mask), beside
    /// the loop. A process that ends doesn't wait for it.
    pub(crate) fn detached_quietly<T: 'static, Fut: Future<Output = T> + 'static>(
        &mut self,
        lane: Lane,
        work: impl FnOnce(Rc<Shared>) -> Fut + 'static,
        finish: impl FnOnce(&mut App, T) + 'static,
    ) {
        self.detached.quiet_running += 1;
        self.spawn_detached(lane, false, work, move |app, result| {
            app.detached.quiet_running = app.detached.quiet_running.saturating_sub(1);
            finish(app, result);
            None
        });
    }

    /// What is finished, if it still counts: the reply to send, if any.
    pub(crate) fn finish_detached(&mut self, finished: Finished) -> Option<Reply> {
        if finished.round != self.detached.round {
            return None;
        }
        (finished.finish)(self)
    }

    /// Drops what nobody is waiting for: the process is ending.
    pub(crate) fn drop_quiet_work(&mut self) {
        for work in self.detached.running.iter().filter(|work| !work.replies) {
            work.task.abort();
        }
        self.detached.maintenance_running = false;
        self.detached.quiet_running = 0;
    }

    /// Drops all of it, with what has finished and is still to be looked at, and the tasks of group
    /// calls, which run beside the lanes: the ID is going, and nothing written or sent on its
    /// behalf may follow.
    pub(crate) fn drop_detached_work(&mut self) {
        for work in self.detached.running.drain(..) {
            work.task.abort();
        }
        self.group_call_tasks.abort_all();
        self.group_calls_watched.borrow_mut().clear();
        self.detached.round += 1;
        self.detached.replies_due = 0;
        self.detached.maintenance_running = false;
        self.detached.quiet_running = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::app::StartConfig;

    fn app(dir: &std::path::Path) -> App {
        App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn work_in_turn_keeps_its_order_and_free_work_does_not_wait() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = app(dir.path());
                let mut done = app.take_detached_work().unwrap();
                let (release, released) = tokio::sync::oneshot::channel::<()>();
                let order = Rc::new(std::cell::RefCell::new(Vec::new()));
                let log = |name: &'static str| {
                    let order = Rc::clone(&order);
                    move |_: &mut App, ()| order.borrow_mut().push(name)
                };
                app.detached_quietly(
                    Lane::InTurn,
                    |_| async move {
                        let _ = released.await;
                    },
                    log("first"),
                );
                app.detached_quietly(Lane::InTurn, |_| async {}, log("second"));
                app.detached_quietly(Lane::Free, |_| async {}, log("free"));
                let finished = done.recv().await.unwrap();
                assert!(app.finish_detached(finished).is_none());
                assert_eq!(*order.borrow(), ["free"]);
                release.send(()).unwrap();
                for _ in 0..2 {
                    let finished = done.recv().await.unwrap();
                    assert!(app.finish_detached(finished).is_none());
                }
                assert_eq!(*order.borrow(), ["free", "first", "second"]);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_is_dropped_for_a_removal_does_nothing_more() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = app(dir.path());
                let mut done = app.take_detached_work().unwrap();
                let touched = Rc::new(std::cell::Cell::new(0));
                let touch = || {
                    let touched = Rc::clone(&touched);
                    move |_: &mut App, ()| touched.set(touched.get() + 1)
                };
                // Through already, and waiting for the loop to look at it.
                app.detached_quietly(Lane::Free, |_| async {}, touch());
                tokio::task::yield_now().await;
                // Under way.
                app.detached_quietly(Lane::Free, |_| std::future::pending::<()>(), touch());
                tokio::task::yield_now().await;
                app.drop_detached_work();
                let finished = done.recv().await.unwrap();
                assert!(app.finish_detached(finished).is_none());
                assert_eq!(touched.get(), 0);
                let nothing = tokio::time::timeout(Duration::from_millis(50), done.recv()).await;
                assert!(nothing.is_err(), "the one under way is gone");
            })
            .await;
    }

    /// Only the UI's commands go beside the loop: Threema Web needs the answer in place.
    #[tokio::test(flavor = "current_thread")]
    async fn work_asked_for_by_others_is_done_in_place() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let mut app = app(dir.path());
                let outcome = app
                    .detached(Lane::InTurn, |_| async { Ok(json!({"here": true})) })
                    .await
                    .unwrap();
                assert_eq!(outcome["here"], true);
                assert!(!app.reply_later);
            })
            .await;
    }
}
