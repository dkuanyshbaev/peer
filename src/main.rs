use clap::Clap;
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{
    future,
    future::{select, Either},
    pin_mut,
    stream::TryStreamExt,
    SinkExt, StreamExt, TryFutureExt,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, connect_async, WebSocketStream};
use tungstenite::protocol::Message;
use url::Url;

type Tx = UnboundedSender<Message>;
type Peers = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

#[derive(Clap)]
struct Opts {
    #[clap(long)]
    period: u64,
    #[clap(long)]
    port: u64,
    #[clap(long)]
    connect: Option<String>,
}

async fn handler(peers: Peers, stream: TcpStream, address: SocketAddr, period: u64) {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Failed to accept");

    let (tx, rx) = unbounded();
    peers.lock().unwrap().insert(address, tx);

    let (ws_sender, mut ws_receiver) = ws_stream.split();
    let mut interval = tokio::time::interval(Duration::from_secs(period));

    let mut message_future = ws_receiver.next();
    let mut tick_future = interval.next();

    // TODO: send peers list to client or add to message header

    loop {
        match select(message_future, tick_future).await {
            Either::Left((message, tick_fut_continue)) => {
                match message {
                    Some(message) => {
                        if let Ok(message) = message {
                            if message.is_close() {
                                break;
                            }
                            println!("Received message {} from {}", message, address);
                        };

                        tick_future = tick_fut_continue;
                        message_future = ws_receiver.next();
                    }
                    None => break,
                };
            }
            Either::Right((_, msg_fut_continue)) => {
                let peers = peers.lock().unwrap();
                for (a, r) in peers.iter() {
                    println!("Sending message Hello! to {}", a);
                    r.unbounded_send(Message::Text("Hello!".to_owned()))
                        .unwrap();
                }

                message_future = msg_fut_continue;
                tick_future = interval.next();
            }
        }
    }

    println!("{} disconnected", &address);
    peers.lock().unwrap().remove(&address);
}

#[tokio::main]
async fn main() {
    let opts: Opts = Opts::parse();

    let peers = Peers::new(Mutex::new(HashMap::new()));
    let my_address = format!("127.0.0.1:{}", opts.port);

    match opts.connect {
        // as client
        Some(server_address) => {
            let url = Url::parse(format!("ws://{}", server_address).as_ref()).unwrap();
            let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
            println!("My address is {}", my_address);

            // let (tx, rx) = unbounded();
            // let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded::<Message>();

            let (mut ws_sender, mut ws_receiver) = ws_stream.split();
            let mut interval = tokio::time::interval(Duration::from_secs(opts.period));

            let mut message_future = ws_receiver.next();
            let mut tick_future = interval.next();

            loop {
                match select(message_future, tick_future).await {
                    Either::Left((message, tick_fut_continue)) => {
                        match message {
                            Some(message) => {
                                if let Ok(message) = message {
                                    if message.is_close() {
                                        break;
                                    }
                                    println!("Received message {} from {}", message, "????");
                                };

                                tick_future = tick_fut_continue;
                                message_future = ws_receiver.next();
                            }
                            None => break,
                        };
                    }
                    Either::Right((_, msg_fut_continue)) => {
                        println!("time to send a message!");
                        // let peers = peers.lock().unwrap();
                        // for (_, r) in peers.iter() {
                        //     r.unbounded_send(Message::Text("Hi!".to_owned())).unwrap();
                        // }

                        ws_sender
                            .send(tungstenite::Message::Text("Hi!".to_string()))
                            .map_err(|e| ())
                            .await;

                        message_future = msg_fut_continue;
                        tick_future = interval.next();
                    }
                }
            }
        }
        // as server
        None => {
            let listener = TcpListener::bind(&my_address).await.expect("Can't listen");
            println!("My address is {}", my_address);

            while let Ok((stream, address)) = listener.accept().await {
                tokio::spawn(handler(peers.clone(), stream, address, opts.period));
            }
        }
    }
}
