//!  # cg-local-rs
//!
//! A background process for syncing [Codingame IDE](https://www.codingame.com/) with a local file.
//! It allows using your preferred IDE for solving puzzles.
//!
//! This app is compatible with browser extentions
//! * [Firefox](https://addons.mozilla.org/en-US/firefox/addon/cg-local/)
//! * [Chrome](https://chrome.google.com/webstore/detail/cg-local/ihakjfajoihlncbnggmcmmeabclpfdgo)
//!
//! **cg-local-rs** is a replacement for a  [CG Local App](https://github.com/jmerle/cg-local-app) which is required an old version of Java
//! and the old version of openjfx and might be hard to install.
//!
//! Original thread about CG Local is here https://www.codingame.com/forum/t/cg-local/10359
//!
//! ## Installation
//!
//! ### Using cargo
//! ```
//! $ cargo install cg-local-rs
//! ```
//!
//! ### From the prebuilt release
//!
//! Download the latest release
//! https://github.com/e-max/cg-local-rs/releases
//!
//!
//!
//! ## Usage
//! First, you need to install an extension for your browser [Firefox](https://addons.mozilla.org/en-US/firefox/addon/cg-local/), [Chrome](https://chrome.google.com/webstore/detail/cg-local/ihakjfajoihlncbnggmcmmeabclpfdgo).
//!
//!
//! Then you need to choose a local file you want to synchronize with CodiGame and pass the name to **cg-local--rs**
//! ```
//! $ cg-local-rs ./src/main.rs
//! ```
//! And finally you need to connect to this app from your CodinGame IDE. You can do it on a page with a pazzle
//!
//!
//! That's all!  All changes you make in your file will be immediately uploaded to CodinGame IDE.
//!
//! We consider your local file state superior to CodinGame IDE state and sync only in one direction - from your local file to CodinGame IDE.
//!
//! There are two exceptions:
//! * if the local file is empty when you run the tool, it will download state from CodinGame IDE.
//! * when you start the tool with a flag **--force-first-download**
//!
//!
//! I use it mostly with Firefox extension and it works pretty well. If you experience any issue, I'll appreciate if you create an issue.

use futures::TryFutureExt;
use futures::{pin_mut, select, try_join, FutureExt, Sink, SinkExt, Stream, StreamExt};
use log::{self, debug, error, info, warn};
use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use serde;
use serde::{Deserialize, Serialize};
use serde_json;
use simplelog::{Config, TermLogger, TerminalMode};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc as blocking_mpsc;
use std::sync::Arc;
use std::time::Duration;
use structopt::StructOpt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::accept_async;
use tungstenite::protocol::Message;
use tungstenite::Error as WsError;

#[doc(hidden)]
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "action", content = "payload")]
enum Msg {
    #[serde(rename = "send-details")]
    SendDetails {},
    #[serde(rename = "details")]
    Details {
        title: String,
        #[serde(rename = "questionId")]
        question_id: u32,
    },
    #[serde(rename = "app-ready")]
    AppReady {},
    #[serde(rename = "send-code")]
    SendCode {},
    #[serde(rename = "code")]
    Code { code: String },
    #[serde(rename = "update-code")]
    UpdateCode { code: String, play: bool },
}

#[doc(hidden)]
#[derive(PartialEq, Clone, Debug)]
struct Question {
    question_id: u32,
    title: String,
}

#[doc(hidden)]
#[derive(Clone)]
struct Monitor {
    path: PathBuf,
    associated_question: Option<Question>,
    bcast: broadcast::Sender<()>,
    force_first_download: bool,
}

#[doc(hidden)]
#[derive(StructOpt, Debug)]
#[structopt(name = "basic")]
struct Opt {
    ///Download file from the service in the beginning
    #[structopt(long)]
    force_first_download: bool,

    ///Debug info
    #[structopt(long)]
    debug: bool,

    /// File to process
    #[structopt(name = "FILE", parse(from_os_str))]
    file: PathBuf,
}

#[doc(hidden)]
#[tokio::main]
async fn main() -> Result<(), Error> {
    let opt = Opt::from_args();
    let loglevel = if opt.debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let _ = TermLogger::init(loglevel, Config::default(), TerminalMode::Mixed);

    let path = opt.file;
    let (tx, _) = broadcast::channel::<()>(16);

    //let (mut quit_tx, mut quit_rx) = oneshot::channel::<()>();

    let handle = tokio::task::spawn_blocking({
        let path = path.clone();
        let tx = tx.clone();
        move || monitor_file(path, tx).map_err(|e| error!("{:?}", e))
    });

    let monitor = Arc::new(RwLock::new(Monitor {
        path: path.clone(),
        associated_question: None,
        bcast: tx,
        force_first_download: opt.force_first_download,
    }));

    let addr = "localhost:53135";
    let mut listener = TcpListener::bind(addr).await?;
    info!("Listening on: {}", addr);

    let listener = async {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(
                accept_connection(stream, monitor.clone())
                    .map_err(|e| error!("Failed with error: {:?}", e)),
            );
        }
    }
    .fuse();

    let file_monitor = handle.fuse();
    pin_mut!(listener, file_monitor);
    select! {
    _ = listener => (),
    _ = file_monitor => ()};

    Ok(())
}

#[doc(hidden)]
fn monitor_file<P: AsRef<Path> + Clone>(
    path: P,
    bcast: broadcast::Sender<()>,
) -> Result<(), Error> {
    info!("Start monitoring file {}", path.as_ref().display());
    'main: loop {
        if !path.as_ref().exists() {
            return Err(Error::FileMonitor(format!(
                "File {} doesn't exists",
                path.as_ref().display()
            )));
        }
        let (tx, rx) = blocking_mpsc::channel();
        let mut watcher = watcher(tx, Duration::from_secs(1)).unwrap();
        watcher
            .watch(path.clone(), RecursiveMode::Recursive)
            .unwrap();

        loop {
            let ev = rx
                .recv()
                .map_err(|e| Error::FileMonitor(format!("notify error {}", e)))?;
            debug!("inotify event {:?}", ev);
            match ev {
                DebouncedEvent::NoticeRemove(_) => {
                    // Well file either removed or just an editor use Write and Move strategy
                    // (like vim does). Check it
                    if !path.as_ref().exists() {
                        return Err(Error::FileMonitor("File removed".to_owned()));
                    }

                    info!("File updated");
                    tokio::spawn({
                        let bcast = bcast.clone();
                        || async move { bcast.send(()) }
                    }());

                    //everything seems fine. Let start from the beginning
                    continue 'main;
                }
                DebouncedEvent::NoticeWrite(_) | DebouncedEvent::Write(_) => {
                    info!("File updated");
                    tokio::spawn({
                        let bcast = bcast.clone();
                        || async move { bcast.send(()) }
                    }());
                }
                DebouncedEvent::Chmod(_) => {
                    info!("Chmod on file");
                }
                _ => return Err(Error::FileMonitor(format!("{:?}", ev))),
            };
        }
    }
}

#[doc(hidden)]
async fn accept_connection(stream: TcpStream, monitor: Arc<RwLock<Monitor>>) -> Result<(), Error> {
    let addr = stream.peer_addr()?;
    debug!("addr {}", addr);
    let ws_stream = accept_async(stream).await?;

    let (writer, reader) = ws_stream.split();
    let (mut tx, rx) = mpsc::channel::<Msg>(10);

    info!("Got connection from browser");

    debug!(" Send initial command");
    tx.send(Msg::SendDetails {}).await?;
    tokio::spawn({
        let mut tx = tx.clone();
        let monitor = monitor.clone();
        || async move {
            let mut bcast = monitor.read().await.bcast.subscribe();
            while let Ok(_) = bcast.recv().await {
                debug!("File updated. Try to upload");
                let mon = monitor.read().await;
                if mon.associated_question.is_none() {
                    warn!("no associated question yet");
                    continue;
                }
                let path = &mon.path;
                let res = fs::metadata(path);
                if res.is_err() {
                    warn!("got error {:?}", res);
                    continue;
                }
                if res.unwrap().len() == 0 {
                    warn!("file size is 0. ignore");
                }
                if let Ok(msg) = fs::read(path)
                    .map_err(|e: io::Error| format!("Cannot read file {}", e))
                    .and_then(|body| {
                        String::from_utf8(body).map_err(|_| "cannot read file as string".to_owned())
                    })
                    .map(|code| Msg::UpdateCode { code, play: false })
                {
                    tx.send(msg).await?;
                }
            }
            Ok::<(), Error>(())
        }
    }());

    try_join!(
        handle_incoming(tx, reader, monitor),
        handle_outgoing(rx, writer)
    )
    .map(|_| ())
}

#[doc(hidden)]
async fn handle_outgoing<S>(mut rx: mpsc::Receiver<Msg>, mut writer: S) -> Result<(), Error>
where
    S: Sink<Message> + Unpin,
{
    while let Some(msg) = rx.recv().await {
        debug!("Try to send {:?}", msg);
        let resp = serde_json::to_string(&msg)?;
        writer
            .send(Message::text(resp))
            .await
            .map_err(|_| Error::General("Some error".to_owned()))?;
    }

    Ok(())
}

#[doc(hidden)]
async fn handle_incoming<S>(
    mut tx: mpsc::Sender<Msg>,
    mut reader: S,
    monitor: Arc<RwLock<Monitor>>,
) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, WsError>> + Unpin,
{
    while let Some(msg) = reader.next().await {
        let msg = msg?;
        debug!("receive raw msg = {:?}", msg);
        match msg {
            Message::Text(txt) => handle_message(&mut tx, &txt, monitor.clone()).await?,
            _ => error!("Unknown WebSocket message type {:?}", msg),
        }
    }
    Ok(())
}

#[doc(hidden)]
async fn handle_message(
    tx: &mut mpsc::Sender<Msg>,
    s: &str,
    monitor: Arc<RwLock<Monitor>>,
) -> Result<(), Error> {
    let m: Msg = serde_json::from_str(s)?;
    debug!(" got message = {:?}", m);
    match m {
        Msg::Details {
            ref title,
            question_id,
        } => handle_details(title, question_id, monitor, tx).await,
        Msg::Code { ref code } => handle_code(code, monitor).await,
        _ => Err(Error::UnknownMessage(m)),
    }
}

#[doc(hidden)]
async fn handle_code(code: &str, monitor: Arc<RwLock<Monitor>>) -> Result<(), Error> {
    let mut monitor = monitor.write().await;
    let mdata = fs::metadata(&monitor.path)?;
    if mdata.len() == 0 || monitor.force_first_download {
        info!("Download file from server");
        fs::write(&monitor.path, code.as_bytes())?;
        monitor.force_first_download = false;
    } else {
        warn!("File is not empty. We won't overwrite it with a version from server");
    }
    Ok(())
}

#[doc(hidden)]
async fn handle_details(
    title: &str,
    question_id: u32,
    monitor: Arc<RwLock<Monitor>>,
    tx: &mut mpsc::Sender<Msg>,
) -> Result<(), Error> {
    let mut monitor = monitor.write().await;
    let new_question = Question {
        question_id,
        title: title.to_owned(),
    };

    if let Some(ref q) = monitor.associated_question {
        if q != &new_question {
            return Err(Error::QuestionConflict {
                current_question: q.title.clone(),
                new_question: new_question.title.clone(),
            });
        }
    }

    monitor.associated_question = Some(new_question);
    tx.send(Msg::AppReady {}).await.map_err(|e| e.into())
}

#[doc(hidden)]
#[derive(Debug)]
enum Error {
    WebSocket(WsError),
    Json(serde_json::Error),
    IO(io::Error),
    General(String),
    Send(mpsc::error::SendError<Msg>),
    UnknownMessage(Msg),
    FileMonitor(String),
    QuestionConflict {
        current_question: String,
        new_question: String,
    },
}

impl From<WsError> for Error {
    fn from(e: WsError) -> Error {
        Error::WebSocket(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IO(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<mpsc::error::SendError<Msg>> for Error {
    fn from(e: mpsc::error::SendError<Msg>) -> Self {
        Error::Send(e)
    }
}
