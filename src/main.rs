use clap::Clap;
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{
    // future,
    future::{select, Either},
    // pin_mut,
    // stream::TryStreamExt,
    // SinkExt,
    StreamExt,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
// use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
// use tokio_tungstenite::{accept_async, connect_async, tungstenite::Error};
use tokio_tungstenite::connect_async;
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

async fn server(peers: Peers, stream: TcpStream, address: SocketAddr, period: u64) {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Failed to accept");

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peers.lock().unwrap().insert(address, tx);

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut interval = tokio::time::interval(Duration::from_secs(period));

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
                let broadcast_recipients = peers
                    .iter()
                    .filter(|(peer_addr, _)| peer_addr != &&address)
                    .map(|(_, ws_sink)| ws_sink);

                for r in broadcast_recipients {
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

async fn client(peers: Peers, stream: WebSocketStream, address: SocketAddr, period: u64) {
    //----------------------------------
}

#[tokio::main]
async fn main() {
    let opts: Opts = Opts::parse();

    let peers = Peers::new(Mutex::new(HashMap::new()));

    match opts.connect {
        // as client
        Some(server_address) => {
            let url = Url::parse(format!("ws://{}", server_address).as_ref()).unwrap();
            let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
            println!("My address is 127.0.0.1:{}", opts.port);

            // tokio::spawn(client(
            //     peers.clone(),
            //     ws_stream,
            //     server_address,
            //     opts.period,
            // ));
        }
        // as server
        None => {
            let my_address = format!("127.0.0.1:{}", opts.port);
            let listener = TcpListener::bind(&my_address).await.expect("Can't listen");
            println!("My address is {}", my_address);

            while let Ok((stream, address)) = listener.accept().await {
                tokio::spawn(server(peers.clone(), stream, address, opts.period));
            }
        }
    }
}
