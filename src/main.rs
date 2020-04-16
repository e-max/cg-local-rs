use futures::{SinkExt, StreamExt};
use log::{error, info};
use std::io::{Error, ErrorKind};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};
use tungstenite::protocol::Message;
use tungstenite::Result as WsResult;

const send_details: &'static str = r#"{"payload":{},"action":"send-details"}"#;

enum Msg {
    SendDetails,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    println!("Hello, world!");
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
