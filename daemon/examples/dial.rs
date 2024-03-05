use anyhow::Result;
use krata::control::{ListRequest, Message, Request, RequestBox};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use tokio_stream::{wrappers::LinesStream, StreamExt};

#[tokio::main]
async fn main() -> Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:4050").await?;
    let (read, mut write) = stream.split();
    let mut read = LinesStream::new(BufReader::new(read).lines());

    let send = Message::Request(RequestBox {
        id: 1,
        request: Request::List(ListRequest {}),
    });
    let mut line = serde_json::to_string(&send)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    println!("sent: {:?}", send);
    while let Some(line) = read.try_next().await? {
        let message: Message = serde_json::from_str(&line)?;
        println!("received: {:?}", message);
    }
    Ok(())
}
