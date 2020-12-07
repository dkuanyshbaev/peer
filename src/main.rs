//! Test task.
//!
//! This is a simple p2p application which can connect itself to the other peers.
//! Once connected, each peer sends a message to all the other peers every N seconds.
//! When a peer receives a message from the other peers, it prints it to the console.
//!
//! Each peer can act as a server or as a client - depending on cli options:
//! if "--connected" option is present - start as a client
//! otherwise - start as a server.
//!
//! Example run:
//! peer --period=4 --port=8080
//! peer --period=2 --port=8081 --connect="127.0.0.1:8080"
//!
//! Known issues:
//! - wrong ports in log
//! - show full list of peers on message send

use clap::Clap;
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{
    future,
    future::{select, Either},
    stream::TryStreamExt,
    SinkExt, StreamExt,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, connect_async, WebSocketStream};
use tungstenite::protocol::Message;
use url::Url;

// peers map
type Tx = UnboundedSender<Message>;
type Peers = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

// cli params where:
// period - message sending period
// port - peer port
// connect - optional address of the peer to connect
// example: peer --period=4 --port=8081 --connect="127.0.0.1:8080"
#[derive(Clap)]
struct Opts {
    #[clap(long)]
    period: u64,
    #[clap(long)]
    port: u64,
    #[clap(long)]
    connect: Option<String>,
}

#[tokio::main]
async fn main() {
    let opts: Opts = Opts::parse();
    let my_address = format!("127.0.0.1:{}", opts.port);
    println!("My address is {}", my_address);

    let peers = Peers::new(Mutex::new(HashMap::new()));

    // with "--connect" option start as a client
    // otherwise start as a server
    match opts.connect {
        // as a client
        Some(server_address) => {
            // connect to the TCP listener
            let url = Url::parse(format!("ws://{}", server_address).as_ref()).unwrap();
            let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
            client_handler(ws_stream, opts.period, opts.port).await;
        }
        // as a server
        None => {
            // create the TCP listener we'll accept connections on
            // and spawn the handling of each connection in a separate task
            let listener = TcpListener::bind(&my_address).await.expect("Can't listen");
            while let Ok((stream, address)) = listener.accept().await {
                tokio::spawn(server_handler(
                    peers.clone(),
                    stream,
                    address,
                    opts.period,
                    my_address.clone(),
                ));
            }
        }
    }
}

async fn server_handler(
    peers: Peers,
    stream: TcpStream,
    address: SocketAddr,
    period: u64,
    server_address: String,
) {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    let (mut ws_sender, ws_receiver) = ws_stream.split();

    // send list of known peers in first message to each client
    let mut peers_list = format!("peers:{}", server_address);
    for (a, _) in peers.lock().unwrap().iter() {
        peers_list = format!("{},{}", peers_list, a);
    }
    ws_sender
        .send(Message::Text(peers_list.clone()))
        .await
        .unwrap();

    // save "write" part of client channel to the peer map
    let (tx, rx) = unbounded();
    peers.lock().unwrap().insert(address, tx);

    // print incoming messages and broadcast them to all known peers
    let mut broadcast_incoming = ws_receiver.try_for_each(|message| {
        println!("Received message {}", message);

        let peers = peers.lock().unwrap();
        let broadcast_recipients = peers
            .iter()
            .filter(|(peer, _)| peer != &&address)
            .map(|(_, ws_sink)| ws_sink);

        for recp in broadcast_recipients {
            recp.unbounded_send(message.clone()).unwrap();
        }

        future::ok(())
    });

    let mut receive_from_others = rx.map(Ok).forward(ws_sender);

    // time interval for message sending
    let mut interval = tokio::time::interval(Duration::from_secs(period));
    let mut tick_future = interval.next();

    loop {
        // check which branch will completes first
        tokio::select! {
            _ = &mut tick_future => {
                // as a server we will send "Hello!" message
                println!("Sending message Hello! to {}", "all");
                for (_, r) in peers.lock().unwrap().iter() {
                    r.unbounded_send(Message::Text(format!("Hello! from {}", server_address))).unwrap();
                };
            }
            _ = &mut broadcast_incoming => {
            }
            _ = &mut receive_from_others => {
            }
        }
    }
}

async fn client_handler(ws_stream: WebSocketStream<TcpStream>, period: u64, port: u64) {
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // receive incoming
    let mut message_future = ws_receiver.next();
    // time interval for message sending
    let mut interval = tokio::time::interval(Duration::from_secs(period));
    let mut tick_future = interval.next();

    loop {
        // as a client we need to print incoming messages
        // and send our message to the server
        match select(message_future, tick_future).await {
            // we have a message
            Either::Left((message, tick_fut_continue)) => {
                match message {
                    Some(message) => {
                        if let Ok(message) = message {
                            if message.is_close() {
                                break;
                            }
                            // the firs message of each connection is the list of known peers
                            // TODO: a message for update peers list
                            if message.to_string().contains("peers") {
                                // get list of peers from string
                                let buf: String = message.to_string().drain(6..).collect();
                                let peers_list: Vec<&str> = buf.split_terminator(",").collect();
                                println!("Connected to peers at {:?}", peers_list);
                            } else {
                                println!("Received message {}", message);
                            }
                        };

                        // continue waiting for tick
                        tick_future = tick_fut_continue;
                        // next message
                        message_future = ws_receiver.next();
                    }
                    None => break,
                };
            }
            // time to send our message
            Either::Right((_, msg_fut_continue)) => {
                // as a client we will send "Hi!" message
                println!("Sending message Hi! to {}", "all");
                let m = format!("Hi! from 127.0.0.1:{}", port);
                ws_sender.send(Message::Text(m)).await.unwrap();

                // continue receiving the messages
                message_future = msg_fut_continue;
                // next tick
                tick_future = interval.next();
            }
        }
    }
}
