//! Human host-browser controls never enter model-visible conversation history.
use super::*;
use crate::process_client::host_browser::{Control, Handle};
use anyhow::{Context, Result, ensure};

impl App {
    pub(super) fn browser_command(&mut self, target: Target, command: &str) -> Result<()> {
        let action = command.strip_prefix("/browser").unwrap_or("").trim();
        match action {
            "" | "open" => {
                ensure!(
                    self.clients.available(target.route),
                    "Connect this Vessel before opening a browser viewer"
                );
                let view = self
                    .views
                    .get(&target)
                    .context("Selected voyage unavailable")?;
                ensure!(
                    !view.archived() && !view.deleted(),
                    "Restore the voyage before opening its browser"
                );
                ensure!(
                    !self.browsers.get(&target).is_some_and(|h| h.finished()),
                    "Previous viewer ended. Use /browser detach before opening a fresh viewer"
                );
                if !self.browsers.contains_key(&target) {
                    ensure!(
                        self.browsers.len()
                            + self
                                .browser_retired
                                .iter()
                                .filter(|job| !job.is_finished())
                                .count()
                            < 4,
                        "At most four browser viewers; detach one first"
                    );
                    let revision = view
                        .snapshot
                        .as_ref()
                        .context("Refresh the voyage before opening its viewer")?
                        .revision;
                    let handle = Handle::start(
                        self.clients[target.route].clone(),
                        target.session,
                        view.process.incarnation,
                        revision,
                    );
                    self.browsers.insert(target, handle);
                    self.browser_opened.remove(&target);
                }
                self.show_browser_panel(target);
            }
            "status" => self.show_browser_panel(target),
            "takeover" | "private" | "agent" | "close" => {
                let control = match action {
                    "takeover" => Control::Human,
                    "private" => Control::Private,
                    "agent" => Control::Agent,
                    _ => Control::Close,
                };
                self.browsers
                    .get(&target)
                    .context("Open and connect this voyage's browser viewer first")?
                    .control(control)?;
                self.status = if action == "close" {
                    "Remote browser close requested; Voyage continues independently"
                } else {
                    "Host browser control requested; remote status is authoritative"
                }
                .into();
            }
            "detach" => {
                if let Some(mut handle) = self.browsers.remove(&target) {
                    handle.stop();
                    self.browser_opened.remove(&target);
                    let sender = self.sender.clone();
                    self.browser_retired.push(tokio::spawn(async move {
                        let result = handle
                            .finish()
                            .await
                            .map(|_| {
                                "Viewer detached; host browser remains owned by Voyage".to_owned()
                            })
                            .map_err(|_| {
                                "Viewer cleanup unresolved; effects were not replayed".to_owned()
                            });
                        let completion = result.clone().map(|_| ());
                        let _ = sender.send(Update::Browser { target, result }).await;
                        completion
                    }));
                }
                self.status =
                    "Detaching viewer. No browser close or Voyage cancellation was sent".into();
            }
            _ => anyhow::bail!(
                "Browser commands: /browser [open|status|takeover|private|agent|close|detach]. Close is remote; detach leaves the host browser running"
            ),
        }
        Ok(())
    }
    fn open_browser_launcher(&mut self, path: std::path::PathBuf) {
        // Only a private file path appears in the opener's arguments, never the fragment credential.
        self.browser_retired.push(tokio::spawn(async move {
            // Opening the ordinary human browser is optional; the private launcher
            // remains available in the panel when the desktop opener is unavailable.
            let _ = crate::process_client::browser::open_launcher(path).await;
            Ok(())
        }));
    }
    pub(super) fn show_browser_panel(&mut self, target: Target) {
        let text = if let Some(handle) = self.browsers.get(&target) {
            let state = handle.state.borrow();
            let launcher = state
                .launcher
                .as_ref()
                .map(|p| {
                    format!(
                        "\n\nOne-use local launcher (open manually if needed):\n{}",
                        safe(&p.display().to_string())
                    )
                })
                .unwrap_or_default();
            format!(
                "# Host browser\n\n{}\n\nBrowser placement: Voyage executing host.\nVoyage: {}\nVessel: {}\n\nF6 opens the shared viewer. Start/Connect, navigation, tabs, private/human/agent control and video are in the viewer.\n\n/browser detach closes only this viewer. /browser close explicitly closes the remote browser. Disconnecting Helm leaves the host browser running. Changing voyages never redirects this viewer. A stale socket refuses effects without replay.{}",
                safe(&state.summary),
                safe(&target.session.to_string()),
                self.route_label(target.route),
                launcher
            )
        } else {
            "# Host browser\n\nNo host browser viewer attached. Press F6 or /browser open. The browser runs on the Voyage host, not this computer. Opening a viewer does not grant agent control; review the explicit controls in the viewer.".into()
        };
        if let Some(view) = self.views.get_mut(&target) {
            view.panel = Some(text);
            view.scroll = 0;
        }
    }
    pub(super) fn poll_browsers(&mut self) {
        let targets = self.browsers.keys().copied().collect::<Vec<_>>();
        for target in targets {
            if self.views.get(&target).is_none_or(|view| {
                !self.browsers[&target].accepts_incarnation(view.process.incarnation)
                    || view.deleted()
                    || view.archived()
            }) {
                self.browsers[&target].stop();
                continue;
            }
            let launcher = self.browsers[&target].state.borrow().launcher.clone();
            if !self.browser_opened.contains(&target)
                && let Some(path) = launcher
            {
                self.browser_opened.insert(target);
                self.open_browser_launcher(path);
            }
            if self
                .views
                .get(&target)
                .and_then(|v| v.panel.as_ref())
                .is_some_and(|p| p.starts_with("# Host browser\n"))
            {
                let changed = self
                    .browsers
                    .get_mut(&target)
                    .is_some_and(|h| h.state.has_changed().unwrap_or(false));
                if changed {
                    self.browsers
                        .get_mut(&target)
                        .unwrap()
                        .state
                        .borrow_and_update();
                    self.show_browser_panel(target);
                }
            }
        }
    }
    pub(super) fn stop_browser_route(&mut self, id: uuid::Uuid) {
        for (target, handle) in &self.browsers {
            if target.route.id == id {
                handle.stop();
            }
        }
    }
    pub(super) async fn finish_browsers(&mut self) -> Result<()> {
        for handle in self.browsers.values() {
            handle.stop();
        }
        let mut failed = false;
        for (_, mut handle) in std::mem::take(&mut self.browsers) {
            failed |= handle.finish().await.is_err();
        }
        for job in self.browser_retired.drain(..) {
            failed |= !matches!(job.await, Ok(Ok(())));
        }
        ensure!(
            !failed,
            "Host browser viewer cleanup remains unresolved; no effects replayed"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod coverage_tests;
