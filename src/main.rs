use futures::{join, SinkExt, Stream, StreamExt};
use log::{error, info};
use serde;
use serde::{Deserialize, Serialize};
use serde_json;
use std::borrow::Cow;
use std::io;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};
use tungstenite::protocol::Message;
use tungstenite::Error as WsError;
use tungstenite::Result as WsResult;

const send_details: &'static str = r#"{"payload":{},"action":"send-details"}"#;
//const send_details2: &'static str = r#"{"payload":{},"action":"SendDetails"}"#;
const details: &'static str =
    r#"{"action":"details","payload":{"title":"The Labyrinth","questionId":16152}}"#;

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
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    println!("Hello, world!");

    let m2 = Msg::Details {
        title: "test".to_owned(),
        question_id: 32,
    };
    let s2 = serde_json::to_string(&m2)?;
    println!("s2 = {:?}", s2);
    let m: Msg = serde_json::from_str(details)?;
    println!("m = {:?}", m);
    let addr = "localhost:53135";
    let mut listener = TcpListener::bind(addr).await?;
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(accept_connection(stream));
    }
    Ok(())
}

async fn accept_connection(stream: TcpStream) -> Result<(), Error> {
    let addr = stream.peer_addr()?;
    info!("addr {}", addr);
    let mut ws_stream = accept_async(stream).await?;
    info!("Send initial message");
    let resp = Message::Text(send_details.to_owned());
    ws_stream.send(resp).await?;
    info!("Start listening");

    let (mut writes, mut reader) = ws_stream.split();

    join!(tokio::spawn(handle_incoming(reader)));

    //read.forward(write)
    //.await
    //.expect("Failed to forward message");

    Ok(())
}

async fn handle_incoming<S>(mut reader: S) -> Result<(), Error>
where
    S: Stream<Item = Result<Message, WsError>> + Unpin,
{
    while let Some(msg) = reader.next().await {
        let msg = msg?;
        println!("msg = {:?}", msg);
        match msg {
            Message::Text(txt) => handle_message(&txt)?,
            _ => (),
        }
    }
    Ok(())
}

fn handle_message(s: &str) -> Result<(), Error> {
    let m: Msg = serde_json::from_str(details)?;
    match m {
        Msg::Details { .. } => {
            println!(" got details = {:?}", m);
        }
        _ => error!("Got unknown message {:?}", m),
    }
    Ok(())
}

#[derive(Debug)]
enum Error {
    WsError(WsError),
    JsonError(serde_json::Error),
    IO(io::Error),
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
