//! Browser commands are local human UI operations, never user-message or model-tool envelopes.
use super::*;
use crate::process_client::browser::{Control, Handle};
use anyhow::{Context, Result, ensure};

impl App {
    pub(super) fn browser_command(&mut self, target: Target, command: &str) -> Result<()> {
        let action=command.strip_prefix("/browser").unwrap_or("").trim();
        match action {
            ""|"open" => {
                ensure!(self.clients.available(target.route),"Connect this Vessel before opening a local browser");
                let view=self.views.get(&target).context("Selected voyage unavailable")?;
                ensure!(!view.archived() && !view.deleted(),"Restore the voyage before sharing a browser");
                ensure!(!self.browsers.get(&target).is_some_and(|h|h.finished()),"Previous browser ended. Use /browser close to observe cleanup before reopening");
                if !self.browsers.contains_key(&target) {
                    ensure!(self.browsers.len()<4,"At most four local browser resources; close one before opening another");
                    let handle=Handle::start(self.clients[target.route].clone(),target.session,view.process.incarnation,
                        format!("{} — {}",view.title(),self.route_label(target.route)));
                    self.browsers.insert(target,handle);
                    self.browser_opened.remove(&target);
                } else if let Some(path)=self.browsers[&target].state.borrow().launcher.clone() {
                    self.open_browser_launcher(path);
                }
                self.show_browser_panel(target);
            }
            "status"=>self.show_browser_panel(target),
            "takeover"|"private" => {
                self.browsers.get(&target).context("Open this voyage's local browser first")?
                    .control(if action=="private"{Control::Private}else{Control::Human})?;
                self.status="Local browser takeover requested. Companion confirms when automation is fenced".into();
            }
            "reconcile"=> {
                ensure!(!self.browsers.get(&target).is_some_and(|h|!h.finished()),"Close the local browser before reconciling its retained cleanup");
                let client=self.clients[target.route].clone();
                let incarnation=self.views.get(&target).context("Selected voyage unavailable")?.process.incarnation;
                let sender=self.sender.clone();
                self.browser_retired.push(tokio::spawn(async move {
                    let result=crate::process_client::browser::reconcile(client,target.session,incarnation).await.map_err(|_|"Browser reconciliation refused or unavailable. Original evidence retained; no effects replayed".into());
                    let _=sender.send(Update::Browser {target,result}).await;
                }));
                self.status="Reconciling observed local cleanup only; uncertain effects are not repeated".into();
            }
            "close"=> {
                if let Some(mut handle)=self.browsers.remove(&target) {
                    handle.stop();
                    self.browser_opened.remove(&target);
                    let sender=self.sender.clone();
                    self.browser_retired.push(tokio::spawn(async move {
                        let result=handle.finish().await.map(|_|"Local browser closed; Voyage continues independently".to_owned())
                            .map_err(|_|"Local browser cleanup or action outcome remains unresolved; retained receipts were not erased".to_owned());
                        let _=sender.send(Update::Browser {target,result}).await;
                    }));
                }
                self.status="Closing this local browser. No Voyage cancellation was sent".into();
            }
            _=>anyhow::bail!("Browser commands: /browser [open|status|takeover|private|close|reconcile]. Return to agent requires explicit review in the local companion"),
        }
        Ok(())
    }
    fn open_browser_launcher(&mut self,path:std::path::PathBuf) {
        // Only a private file path appears in the opener's arguments, never the fragment credential.
        self.browser_retired.push(tokio::spawn(async move {
            let _=crate::process_client::browser::open_launcher(path).await;
        }));
    }
    pub(super) fn show_browser_panel(&mut self,target:Target) {
        let text=if let Some(handle)=self.browsers.get(&target) {
            let state=handle.state.borrow();
            let launcher=state.launcher.as_ref().map(|p|format!("\n\nLocal launcher (open manually if needed):\n{}",safe(&p.display().to_string()))).unwrap_or_default();
            format!("# Local browser\n\n{}\n\nBrowser placement: this computer.\nVoyage: {}\nVessel: {}\n\nThe companion displays the same browser the agent uses. Review local origins, then explicitly Share / Return to agent. Page content can reach this Voyage and its model provider.\n\nF6 opens the companion. /browser takeover or /browser private fences local automation. /browser close closes the owned browser, not the Voyage.\n\nChanging voyages never redirects this browser. Reconnect requires explicit fresh sharing.{}",
                safe(&state.summary),safe(&target.session.to_string()),self.route_label(target.route),launcher)
        } else {
            "# Local browser\n\nNo local browser shared with this voyage. Press F6 or /browser open.\n\nFirst run `helm browser setup` locally. Browser sharing and return-to-agent require explicit local companion consent; provider credentials stay on the Voyage host.".into()
        };
        if let Some(view)=self.views.get_mut(&target) {view.panel=Some(text);view.scroll=0;}
    }
    pub(super) fn poll_browsers(&mut self) {
        let targets=self.browsers.keys().copied().collect::<Vec<_>>();
        for target in targets {
            let launcher=self.browsers[&target].state.borrow().launcher.clone();
            if !self.browser_opened.contains(&target) && let Some(path)=launcher {
                self.browser_opened.insert(target);
                self.open_browser_launcher(path);
            }
            if self.views.get(&target).and_then(|v|v.panel.as_ref()).is_some_and(|p|p.starts_with("# Local browser\n")) {
                let changed=self.browsers.get_mut(&target).is_some_and(|h|h.state.has_changed().unwrap_or(false));
                if changed {
                    self.browsers.get_mut(&target).unwrap().state.borrow_and_update();
                    self.show_browser_panel(target);
                }
            }
        }
    }
    pub(super) fn stop_browser_route(&mut self,id:uuid::Uuid) {
        for (target,handle) in &self.browsers {if target.route.id==id {handle.stop();}}
    }
    pub(super) async fn finish_browsers(&mut self)->Result<()> {
        for handle in self.browsers.values() {handle.stop();}
        let mut failed=false;
        for (_,mut handle) in std::mem::take(&mut self.browsers) {failed|=handle.finish().await.is_err();}
        for job in self.browser_retired.drain(..) {failed|=job.await.is_err();}
        ensure!(!failed,"Local browser cleanup or effects remain unresolved; receipts retained");
        Ok(())
    }
}
