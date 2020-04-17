use env_logger;
use futures::TryFutureExt;
use futures::{try_join, Sink, SinkExt, Stream, StreamExt};
use log::{debug, error, info};
use serde;
use serde::{Deserialize, Serialize};
use serde_json;
use std::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
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
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    info!("Hello, world!");

    let addr = "localhost:53135";
    let mut listener = TcpListener::bind(addr).await?;
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(accept_connection(stream).map_err(|e| error!("Failed with error: {:?}", e)));
    }
    Ok(())
}

async fn accept_connection(stream: TcpStream) -> Result<(), Error> {
    let addr = stream.peer_addr()?;
    info!("addr {}", addr);
    let ws_stream = accept_async(stream).await?;

    let (writer, reader) = ws_stream.split();
    let (mut tx, rx) = mpsc::channel::<Msg>(10);

    info!("Send initial message");

    tx.send(Msg::SendDetails {}).await?;
    info!("Start listening");

    try_join!(handle_incoming(tx, reader), handle_outgoing(rx, writer)).map(|_| ())
}

async fn handle_outgoing<S>(mut rx: mpsc::Receiver<Msg>, mut writer: S) -> Result<(), Error>
where
    S: Sink<Message> + Unpin,
{
    while let Some(msg) = rx.recv().await {
        info!("Try to send {:?}", msg);
        let resp = serde_json::to_string(&msg)?;
        writer
            .send(Message::text(resp))
            .await
            .map_err(|_| Error::General("Some error".to_owned()))?;
    }

    Ok(())
}

async fn handle_incoming<S>(mut tx: mpsc::Sender<Msg>, mut reader: S) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, WsError>> + Unpin,
{
    while let Some(msg) = reader.next().await {
        let msg = msg?;
        debug!("receive raw msg = {:?}", msg);
        match msg {
            Message::Text(txt) => handle_message(&mut tx, &txt).await?,
            _ => error!("Unknown WebSocket message type {:?}", msg),
        }
    }
    Ok(())
}

async fn handle_message(tx: &mut mpsc::Sender<Msg>, s: &str) -> Result<(), Error> {
    let m: Msg = serde_json::from_str(s)?;
    match m {
        Msg::Details { .. } => {
            info!(" got Details message = {:?}", m);
            tx.send(Msg::AppReady {}).await.map_err(|e| e.into())
        }
        _ => Err(Error::UnknownMessage(m)),
    }
}

#[derive(Debug)]
enum Error {
    WsError(WsError),
    JsonError(serde_json::Error),
    IO(io::Error),
    General(String),
    SendError(mpsc::error::SendError<Msg>),
    UnknownMessage(Msg),
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
