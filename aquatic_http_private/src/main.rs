mod workers;
use dotenv::dotenv;

fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let mut handles = Vec::new();

    for _ in 0..2 {
        let handle = ::std::thread::Builder::new()
            .name("socket".into())
            .spawn(move || workers::socket::run_socket_worker())?;

        handles.push(handle);
    }

    for handle in handles {
        handle
            .join()
            .map_err(|err| anyhow::anyhow!("thread join error: {:?}", err))??;
    }

    Ok(())
}
