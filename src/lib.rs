use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use std::io::SeekFrom;
use std::sync::Arc;
use std::path::Path;
use std::fs::remove_file;

use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

/// Formats the sum of two numbers as string.
#[pyfunction]
fn download(url: String, filename: String, max_files: usize, chunk_size: usize) -> PyResult<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async { download_async(url, filename.clone(), max_files, chunk_size).await })
        .map_err(|err| {
            let path = Path::new(&filename);
            if path.exists() {
                match remove_file(filename){
                    Ok(_) => err,
                    Err(err) => {return PyException::new_err(format!("Error while removing corrupted file: {err:?}"));}
                }
            }else{
                err
            }
        })
}

async fn download_async(
    url: String,
    filename: String,
    max_files: usize,
    chunk_size: usize,
) -> PyResult<()> {
    // let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    let response = client
        .head(&url)
        .send()
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    let length = response
        .headers()
        .get(CONTENT_LENGTH)
        .ok_or(PyException::new_err("No content length"))?
        .to_str()
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    let length: usize = length
        .parse()
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;

    let mut handles = vec![];
    let semaphore = Arc::new(Semaphore::new(max_files));

    let chunk_size = chunk_size;
    for start in (0..length).step_by(chunk_size) {
        let url = url.clone();
        let filename = filename.clone();
        let client = client.clone();

        let stop = std::cmp::min(start + chunk_size - 1, length);
        let permit =
            semaphore.clone().acquire_owned().await.map_err(|err| {
                PyException::new_err(format!("Error while downloading: {:?}", err))
            })?;
        handles.push(tokio::spawn(async move {
            let chunk = download_chunk(client, url, filename, start, stop).await;
            drop(permit);
            chunk
        }));
    }

    // Output the chained result
    let results: Vec<Result<PyResult<()>, tokio::task::JoinError>> =
        futures::future::join_all(handles).await;
    let results: PyResult<()> = results.into_iter().flatten().collect();
    let _ = results?;

    // let size = length as f64 / 1024.0 / 1024.0;
    // let speed = size / start.elapsed().as_secs_f64();
    // println!(
    //     "Took {:?} for {:.2}Mo ({:.2} Mo/s)",
    //     start.elapsed(),
    //     size,
    //     speed
    // );
    Ok(())
}

async fn download_chunk(
    client: reqwest::Client,
    url: String,
    filename: String,
    start: usize,
    stop: usize,
) -> PyResult<()> {
    // Process each socket concurrently.
    let range = format!("bytes={start}-{stop}");
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(filename)
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    file.seek(SeekFrom::Start(start as u64))
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    let response = client
        .get(url)
        .header(RANGE, range)
        .send()
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    let content = response
        .bytes()
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    file.write_all(&content)
        .await
        .map_err(|err| PyException::new_err(format!("Error while downloading: {:?}", err)))?;
    Ok(())
}

/// A Python module implemented in Rust.
#[pymodule]
fn hf_transfer(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(download, m)?)?;
    Ok(())
}
