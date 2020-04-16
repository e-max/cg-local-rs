use futures::{SinkExt, StreamExt};
use log::{error, info};
use serde;
use serde::{Deserialize, Serialize};
use serde_json;
use std::io::{Error, ErrorKind};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};
use tungstenite::protocol::Message;
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

async fn accept_connection(stream: TcpStream) -> WsResult<()> {
    let addr = stream.peer_addr()?;
    info!("addr {}", addr);
    let mut ws_stream = accept_async(stream).await?;
    info!("Send initial message");
    let resp = Message::Text(send_details.to_owned());
    ws_stream.send(resp).await?;
    info!("Start listening");

    while let Some(msg) = ws_stream.next().await {
        let msg = msg?;
        println!("msg = {:?}", msg);
        if msg.is_text() || msg.is_binary() {
            //ws_stream.send(msg).await?;
            println!(" msg is text of binary");
        }
    }

    //read.forward(write)
    //.await
    //.expect("Failed to forward message");

    Ok(())
}
