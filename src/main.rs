use clap::Clap;
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt};
use futures_util::{
    future::{select, Either},
    SinkExt, StreamExt,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, connect_async};
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

async fn handler(
    peers: Peers,
    stream: TcpStream,
    address: SocketAddr,
    period: u64,
    server_address: String,
) {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut interval = tokio::time::interval(Duration::from_secs(period));

    // send list of known peers in first message
    let mut peers_list = format!("peers:{}", server_address);
    for (a, _) in peers.lock().unwrap().iter() {
        peers_list = format!("{},{}", peers_list, a);
    }
    ws_sender.send(Message::Text(peers_list)).await.unwrap();

    let (tx, _rx) = unbounded();
    peers.lock().unwrap().insert(address, tx);

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

                            // ------------------------------------------
                            // Forward message
                            // let peers = peers.lock().unwrap();
                            // for (_, r) in peers.iter() {
                            //     r.unbounded_send(message.clone()).unwrap();
                            // }

                            // let peers = peers.lock().unwrap();
                            // let broadcast_recipients = peers
                            //     .iter()
                            //     // .filter(|(peer_addr, _)| peer_addr != &&address)
                            //     .map(|(_, ws_sink)| ws_sink);
                            //
                            // for recp in broadcast_recipients {
                            //     recp.unbounded_send(message.clone()).unwrap();
                            // }

                            // or print
                            println!("Received message '{}' from {}", message, address);
                            // ------------------------------------------
                        };

                        tick_future = tick_fut_continue;
                        message_future = ws_receiver.next();
                    }
                    None => break,
                };
            }
            Either::Right((_, msg_fut_continue)) => {
                println!("Sending message 'Hello!' to {}", address);
                ws_sender
                    .send(Message::Text("Hello!".to_owned()))
                    .await
                    .unwrap();

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
    let my_address = format!("127.0.0.1:{}", opts.port);
    let peers = Peers::new(Mutex::new(HashMap::new()));

    match opts.connect {
        // as client
        Some(server_address) => {
            let url = Url::parse(format!("ws://{}", server_address).as_ref()).unwrap();
            let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
            println!("My address is {}", my_address);

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
                                    if message.to_string().contains("peers") {
                                        // get list of peers from string
                                        let buf: String = message.to_string().drain(6..).collect();
                                        let peers_list: Vec<&str> =
                                            buf.split_terminator(",").collect();
                                        println!("Connected to the peers at {:?}", peers_list);
                                    } else {
                                        println!(
                                            "Received message '{}' from {}",
                                            message, server_address
                                        );
                                    }
                                };

                                tick_future = tick_fut_continue;
                                message_future = ws_receiver.next();
                            }
                            None => break,
                        };
                    }
                    Either::Right((_, msg_fut_continue)) => {
                        println!("Sending message 'Hi!' to {}", server_address);
                        // ws_sender
                        //     .send(Message::Text("Hi!".to_owned()))
                        //     .await
                        //     .unwrap();
                        let m = format!("Hi from {}", opts.port);
                        ws_sender.send(Message::Text(m)).await.unwrap();

                        // TODO connect to all???
                        // let listener = TcpListener::bind(&my_address).await.expect("Can't listen");
                        // while let Ok((stream, address)) = listener.accept().await {
                        //     tokio::spawn(handler(
                        //         peers.clone(),
                        //         stream,
                        //         address,
                        //         opts.period,
                        //         my_address.clone(),
                        //     ));
                        // }

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
                tokio::spawn(handle_connection(
                    peers.clone(),
                    stream,
                    address,
                    my_address.clone(),
                ));
                // tokio::spawn(handler(
                //     peers.clone(),
                //     stream,
                //     address,
                //     opts.period,
                //     my_address.clone(),
                // ));
            }
        }
    }
}

async fn handle_connection(
    peers: Peers,
    stream: TcpStream,
    address: SocketAddr,
    server_address: String,
) {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    let (mut ws_sender, ws_receiver) = ws_stream.split();

    // send list of known peers in first message
    let mut peers_list = format!("peers:{}", server_address);
    for (a, _) in peers.lock().unwrap().iter() {
        peers_list = format!("{},{}", peers_list, a);
    }
    ws_sender.send(Message::Text(peers_list)).await.unwrap();

    let (tx, rx) = unbounded();
    peers.lock().unwrap().insert(address, tx);

    let mut broadcast_incoming = ws_receiver.try_for_each(|message| {
        println!("Received message '{}' from {}", message, address);
        let peers = peers.lock().unwrap();

        let broadcast_recipients = peers
            .iter()
            .filter(|(peer_addr, _)| peer_addr != &&address)
            .map(|(_, ws_sink)| ws_sink);

        for recp in broadcast_recipients {
            recp.unbounded_send(message.clone()).unwrap();
        }

        future::ok(())
    });

    let mut receive_from_others = rx.map(Ok).forward(ws_sender);

    // pin_mut!(broadcast_incoming, receive_from_others);
    // future::select(broadcast_incoming, receive_from_others).await;

    let mut interval = tokio::time::interval(Duration::from_secs(4));
    let mut tick_future = interval.next();
    // pin_mut!(tick_future);

    // let tick_future = tx
    //     .unbounded_send(Message::Text("Hello!".to_owned()))
    //     .unwrap();

    loop {
        tokio::select! {
            _ = &mut tick_future => {
                println!("Sending message 'Hello!' to {}", "all");
                // ws_sender
                //     .send(Message::Text("Hello!".to_owned()))
                //     .await
                //     .unwrap();
                let peers = peers.lock().unwrap();
                let broadcast_recipients = peers
                    .iter()
                    .filter(|(peer_addr, _)| peer_addr != &&address)
                    .map(|(_, ws_sink)| ws_sink);

                for recp in broadcast_recipients {
                    recp.unbounded_send(Message::Text("Hello!".to_owned())).unwrap();
                };
            }
            _ = &mut broadcast_incoming => {
                println!("broadcast_incoming");
            }
            _ = &mut receive_from_others => {
                println!("receive_from_others");
            }
        }
    }

    println!("{} disconnected", &address);
    peers.lock().unwrap().remove(&address);
}
