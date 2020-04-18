use env_logger;
use futures::TryFutureExt;
use futures::{try_join, Sink, SinkExt, Stream, StreamExt};
use log::{debug, error, info, warn};
use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use serde;
use serde::{Deserialize, Serialize};
use serde_json;
use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::mpsc as blocking_mpsc;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::accept_async;
use tungstenite::protocol::Message;
use tungstenite::Error as WsError;

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

#[derive(PartialEq, Clone, Debug)]
struct Question {
    question_id: u32,
    title: String,
}

#[derive(Clone)]
struct Monitor {
    path: String,
    associated_question: Option<Question>,
    bcast: broadcast::Sender<()>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let args = env::args().collect::<Vec<String>>();
    if args.len() < 2 {
        panic!("Must pass a path to a file to monitor");
    }

    let path = args[1].clone();
    let (tx, rx) = broadcast::channel::<()>(16);

    let handle = tokio::task::spawn_blocking({
        let path = path.clone();
        let tx = tx.clone();
        move || monitor_file(path, tx).map_err(|e| panic!("File removed"))
    });

    let monitor = Arc::new(RwLock::new(Monitor {
        path: path.clone(),
        associated_question: None,
        bcast: tx,
    }));

    let addr = "localhost:53135";
    let mut listener = TcpListener::bind(addr).await?;
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(
            accept_connection(stream, monitor.clone())
                .map_err(|e| error!("Failed with error: {:?}", e)),
        );
    }
    Ok(())
}

fn monitor_file<P: AsRef<Path> + Clone>(
    path: P,
    bcast: broadcast::Sender<()>,
) -> Result<(), Error> {
    debug!("Start file monitoring");
    'main: loop {
        if !path.as_ref().exists() {
            panic!("File {} doesn't exists", path.as_ref().display());
        }
        let (mut tx, mut rx) = blocking_mpsc::channel();
        let mut watcher = watcher(tx, Duration::from_secs(1)).unwrap();
        watcher
            .watch(path.clone(), RecursiveMode::Recursive)
            .unwrap();

        loop {
            let ev = rx
                .recv()
                .map_err(|e| Error::FileMonitorError(format!("notify error {}", e)))?;
            debug!("inotify event {:?}", ev);
            match ev {
                DebouncedEvent::NoticeRemove(_) => {
                    // Well file either removed or just an editor use Write and Move strategy
                    // (like vim does). Check it
                    if !path.as_ref().exists() {
                        return Err(Error::FileMonitorError("File removed".to_owned()));
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
                _ => return Err(Error::FileMonitorError(format!("{:?}", ev))),
            };
        }
    }
}

async fn accept_connection(stream: TcpStream, monitor: Arc<RwLock<Monitor>>) -> Result<(), Error> {
    let addr = stream.peer_addr()?;
    info!("addr {}", addr);
    let ws_stream = accept_async(stream).await?;

    let (writer, reader) = ws_stream.split();
    let (mut tx, rx) = mpsc::channel::<Msg>(10);

    info!("Send initial message");

    tx.send(Msg::SendDetails {}).await?;
    info!("Start listening");
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
                        String::from_utf8(body).map_err(|e| "cannot read file as string".to_owned())
                    })
                    .map(|code| Msg::UpdateCode { code, play: false })
                {
                    tx.send(msg).await;
                }
            }
        }
    }());

    try_join!(
        handle_incoming(tx, reader, monitor),
        handle_outgoing(rx, writer)
    )
    .map(|_| ())
}

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

async fn handle_code(code: &str, monitor: Arc<RwLock<Monitor>>) -> Result<(), Error> {
    let path = &monitor.read().await.path;
    let mdata = fs::metadata(path)?;
    if mdata.len() == 0 {
        fs::write(path, code.as_bytes())?;
    } else {
        warn!("File is not empty. We won't overwrite it with a version from server");
    }
    Ok(())
}

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

#[derive(Debug)]
enum Error {
    WsError(WsError),
    JsonError(serde_json::Error),
    IO(io::Error),
    General(String),
    SendError(mpsc::error::SendError<Msg>),
    UnknownMessage(Msg),
    FileMonitorError(String),
    QuestionConflict {
        current_question: String,
        new_question: String,
    },
}

impl From<WsError> for Error {
    fn from(e: WsError) -> Error {
        Error::WsError(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IO(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::JsonError(e)
    }
}

impl From<mpsc::error::SendError<Msg>> for Error {
    fn from(e: mpsc::error::SendError<Msg>) -> Self {
        Error::SendError(e)
    }
}
