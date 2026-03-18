use std::io;

use actor_helper::{Action, Actor, Handle, Receiver, spawn_actor};

struct FragileActor {
    rx: Receiver<Action<FragileActor>>,
}

impl Actor<io::Error> for FragileActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    action(self).await;
                }
                else => break Ok(()),
            }
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let (handle, rx) = Handle::<FragileActor, io::Error>::channel();
    let join_handle = spawn_actor(FragileActor { rx });

    println!("Press Ctrl+C to drop the last handle and shut the actor down.");

    tokio::signal::ctrl_c().await?;
    drop(handle);

    match join_handle.await {
        Ok(Ok(())) => {
            println!("Actor exited cleanly.");
        }
        Ok(Err(error)) => {
            println!("Actor exit was converted into a normal library error:");
            println!("{error}");
        }
        Err(join_error) => {
            eprintln!("Unexpected task panic: {join_error}");
        }
    }

    Ok(())
}
