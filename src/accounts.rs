use crate::git::command;
use eframe::egui::{self, RichText, Ui};
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, PartialEq, Eq, Debug)]
struct Account {
    host: String,
    login: String,
    active: bool,
}

type Loaded = Result<Vec<Account>, String>;

#[derive(Default)]
pub struct Accounts {
    list: Option<Loaded>,
    rx: Option<Receiver<Loaded>>,
    switching: bool,
}

fn parse(text: &str) -> Loaded {
    let fail = || text.lines().next().unwrap_or("No GitHub accounts found.").to_owned();
    let json: serde_json::Value = serde_json::from_str(text).map_err(|_| fail())?;
    let hosts = json["hosts"].as_object().ok_or_else(fail)?;
    let accounts = |host: &String, list: &serde_json::Value| -> Vec<Account> {
        let list = list.as_array().into_iter().flatten();
        list.map(|a| Account {
            host: host.clone(),
            login: a["login"].as_str().unwrap_or_default().to_owned(),
            active: a["active"].as_bool().unwrap_or(false),
        })
        .collect()
    };
    Ok(hosts.iter().flat_map(|(host, list)| accounts(host, list)).collect())
}

impl Accounts {
    fn load(&mut self, switch: Option<Account>, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.switching = switch.is_some();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let dir = Path::new(".");
            let switched = match switch {
                Some(a) => command("gh", dir, &["auth", "switch", "--hostname", &a.host, "--user", &a.login]).map(drop),
                None => Ok(()),
            };
            let list = switched.and_then(|()| {
                let text = command("gh", dir, &["auth", "status", "--json", "hosts"]).unwrap_or_else(|error| error);
                parse(&text)
            });
            let _ = tx.send(list);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> bool {
        let Some(list) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
        self.list = Some(list);
        self.rx = None;
        std::mem::take(&mut self.switching)
    }

    pub fn menu(&mut self, ui: &mut Ui) {
        if self.list.is_none() && self.rx.is_none() {
            self.load(None, ui.ctx());
        }
        ui.set_min_width(220.0);
        ui.label(RichText::new("GITHUB ACCOUNTS").size(11.0).weak());
        let mut switch = None;
        match &self.list {
            None => {
                ui.label("Loading...");
            }
            Some(Err(error)) if error.starts_with("Could not run gh") => {
                ui.label("Install the GitHub CLI (gh) to manage accounts.");
            }
            Some(Err(error)) => {
                ui.label(error);
            }
            Some(Ok(list)) => {
                ui.add_enabled_ui(self.rx.is_none(), |ui| {
                    for account in list {
                        let label = format!("{}  ({})", account.login, account.host);
                        if ui.selectable_label(account.active, label).clicked() && !account.active {
                            switch = Some(account.clone());
                        }
                    }
                });
            }
        }
        ui.separator();
        if ui.button("Refresh").clicked() {
            self.list = None;
        }
        if let Some(account) = switch {
            self.load(Some(account), ui.ctx());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gh_auth_status() {
        let json = r#"{"hosts":{"github.com":[
            {"state":"success","active":true,"host":"github.com","login":"first","tokenSource":"keyring"},
            {"state":"success","active":false,"host":"github.com","login":"second","tokenSource":"keyring"}]}}"#;
        let account = |login: &str, active| Account { host: "github.com".into(), login: login.into(), active };
        assert_eq!(parse(json), Ok(vec![account("first", true), account("second", false)]));
        assert_eq!(parse("You are not logged into any GitHub hosts.\nRun gh auth login"), Err("You are not logged into any GitHub hosts.".into()));
    }
}
