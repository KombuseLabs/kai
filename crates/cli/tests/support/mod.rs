//! An invented hatch-compatible HTTP service. Its file is fixture JSON,
//! never SQLite; no installed program or GitHub credential is involved.
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use types::{Access, BoardFiles, Issue, IssueState, RepoName};

pub struct Store {
    path: PathBuf,
    value: Value,
}
impl Store {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let value = if path.exists() {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            json!({"version":"ticketz-snapshot/1","issues":[],"boards":[]})
        };
        Ok(Self {
            path: path.to_owned(),
            value,
        })
    }
    fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        std::fs::write(&self.path, serde_json::to_vec(&self.value)?)
    }
    pub fn record_sync(
        &mut self,
        board: &RepoName,
        issues: &[Issue],
        _since: Option<&str>,
        full: bool,
    ) -> std::io::Result<()> {
        self.value["issues"]=json!(issues.iter().map(|i|json!({
            "version":"ticketz-issue/1","repo":i.id.repo.to_string(),"number":i.id.number,"title":i.title,"url":i.url,"state":match i.state { IssueState::Open => "open", IssueState::Closed => "closed" },"author":i.author,
            "created_at":i.created_at,"updated_at":i.updated_at,"body":i.body,"labels":i.labels,"labels_complete":i.labels_complete,
            "label_events":i.label_events.iter().map(|e|json!({"label":e.label,"actor":e.actor,"at":e.at})).collect::<Vec<_>>(),"label_events_complete":i.label_events_complete
        })).collect::<Vec<_>>());
        self.value["boards"] = json!([{"version":"ticketz-status/1","repo":board.to_string(),"issues":issues.len(),"last_sync":"2026-01-02T00:00:00Z","last_full_sync":if full {Some("2026-01-02T00:00:00Z")}else{None},"actors":{}}]);
        self.save()
    }
    pub fn record_access(
        &mut self,
        _board: &RepoName,
        answers: &[(String, Access)],
    ) -> std::io::Result<()> {
        let mut actors = serde_json::Map::new();
        for (login, access) in answers {
            let mut v = json!({"observed_at":"2026-01-02T00:00:00Z"});
            match access {
                Access::Known { permission, role } => {
                    v["permission"] = json!(permission);
                    if let Some(role) = role {
                        v["role"] = json!(role);
                    }
                }
                Access::Unknown(why) => v["permission_unknown"] = json!(why),
            }
            actors.insert(login.clone(), v);
        }
        self.value["boards"][0]["actors"] = json!(actors);
        self.save()
    }
    pub fn record_board_files(
        &mut self,
        _board: &RepoName,
        files: &BoardFiles,
    ) -> std::io::Result<()> {
        self.value["boards"][0]["board_files"] = json!({"branch":files.branch,"commit":files.commit,"paths":files.paths,"complete":files.complete,"observed_at":"2026-01-02T00:00:00Z"});
        self.save()
    }
    pub fn issues(&self) -> std::io::Result<Vec<Value>> {
        Ok(self.value["issues"].as_array().unwrap().clone())
    }
}

pub struct Server {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(dir: &Path) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let path = dir.join(".kai/ticketz-fixture.json");
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        len = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let mut fixture = Store::open(&path).unwrap();
                let value = if request.contains("/ticketz/reset ") {
                    let count = fixture.value["issues"].as_array().unwrap().len();
                    fixture.value = json!({"version":"ticketz-snapshot/1","issues":[],"boards":[]});
                    if path.exists() {
                        fixture.save().unwrap();
                    }
                    json!({"version":"ticketz-reset/1","removed":count})
                } else {
                    assert!(request.starts_with("POST /ticketz/snapshot "), "{request}");
                    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({}));
                    fixture.value
                };
                let response =
                    json!({"exit_code":0,"stdout":value.to_string(),"stderr":"","truncated":false})
                        .to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
