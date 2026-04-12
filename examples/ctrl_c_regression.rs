use std::io;

use actor_helper::Handle;

struct FragileActor;

#[tokio::main]
async fn main() -> io::Result<()> {
    let (handle, join_handle) = Handle::<FragileActor, io::Error>::spawn(FragileActor);

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
