use std::io::{self, Write};
use futures::executor;

use std::net::{TcpListener};

fn main() -> std::result::Result<(), std::io::Error> {
    executor::block_on(async {
        let listener = TcpListener::bind("127.0.0.1:8080")?;
        let mut incoming = listener.incoming();

        println!("Listening on 127.0.0.1:8080");

        while let Some(stream) = incoming.next() {
            let mut stream = stream?;

            loony_core::spawn(async move {
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\nHello World!").unwrap();
            });
        }

        Ok(())
    })
}
