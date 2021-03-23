#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_mut)]

use crate::config::Config;
use crate::query::Query;
use crate::util::{create_socket};
use crate::errors;
use crate::errors::ResultExt;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Response, Server, StatusCode};
use tokio::sync::oneshot;
use serde::Serialize;
use serde_json;
use std::collections::HashMap;
use std::num::ParseIntError;
use std::sync::Arc;
use std::thread;
use url::form_urlencoded;
use bitcoin::consensus::encode;
use bitcoin::consensus::encode::serialize;
use bitcoin_hashes::hex::{FromHex, ToHex};
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use bitcoin::hashes::Error as HashError;
use hex::{self, FromHexError};



const TTL_LONG: u32 = 157_784_630; // ttl for static resources (5 years)
const TTL_SHORT: u32 = 10; // ttl for volatile resources
const TTL_MEMPOOL_RECENT: u32 = 5; // ttl for GET /mempool/recent


#[tokio::main]
async fn run_server(config: Arc<Config>, query: Arc<Query>, rx: oneshot::Receiver<()>) {

    let addr = &config.indexer_http_addr;

    let config = Arc::clone(&config);
    let query = Arc::clone(&query);

    let make_service_fn_inn = || {
        let query = Arc::clone(&query);
        let config = Arc::clone(&config);

        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                let query = Arc::clone(&query);
                let config = Arc::clone(&config);

                async move {
                    let method = req.method().clone();
                    let uri = req.uri().clone();
                    let body = hyper::body::to_bytes(req.into_body()).await?;

                    let mut resp = handle_request(method, uri, body, &query, &config)
                        .unwrap_or_else(|err| {
                            warn!("{:?}", err);
                            Response::builder()
                                .status(err.0)
                                .header("Content-Type", "text/plain")
                                .body(Body::from(err.1))
                                .unwrap()
                        });
                    // if let Some(ref origins) = config.cors {
                    //   resp.headers_mut()
                    //     .insert("Access-Control-Allow-Origin", origins.parse().unwrap());
                    // }
                    Ok::<_, hyper::Error>(resp)
                }
            }))
        }
    };

    info!("REST server running on {}", addr);

    let socket = create_socket(&addr);
    socket.listen(511).expect("setting backlog failed");

    let server = Server::from_tcp(socket.into_tcp_listener())
        .expect("Server::from_tcp failed")
        .serve(make_service_fn(move |_| make_service_fn_inn()))
        .with_graceful_shutdown(async {
            rx.await.ok();
        })
        .await;

    if let Err(e) = server {
        eprintln!("server error: {}", e);
    }
}

pub fn start(config: Arc<Config>, query: Arc<Query>) -> Handle {
    let (tx, rx) = oneshot::channel::<()>();

    Handle {
        tx,
        thread: thread::spawn(move || {
            run_server(config, query, rx);
        }),
    }
}

pub struct Handle {
    tx: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
}

impl Handle {
    pub fn stop(self) {
        self.tx.send(()).expect("failed to send shutdown signal");
        self.thread.join().expect("REST server failed");
    }
}

fn json_response<T: Serialize>(value: T, ttl: u32) -> Result<Response<Body>, HttpError> {
    let value = serde_json::to_string(&value)?;
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .header("Cache-Control", format!("public, max-age={:}", ttl))
        .body(Body::from(value))
        .unwrap())
}

fn handle_request(
    method: Method,
    uri: hyper::Uri,
    body: hyper::body::Bytes,
    query: &Query,
    config: &Config,
) -> Result<Response<Body>, HttpError> {
    // TODO it looks hyper does not have routing and query parsing :(
    let path: Vec<&str> = uri.path().split('/').skip(1).collect();
    let query_params = match uri.query() {
        Some(value) => form_urlencoded::parse(&value.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>(),
        None => HashMap::new(),
    };

    info!("handle {:?} {:?}", method, uri);
    match (
        &method,
        path.get(0),
        path.get(1),
        path.get(2),
        path.get(3),
        path.get(4),
    ) {

        (&Method::GET, Some(&"blockchain"), Some(&"scripthash"), Some(script_str), Some(&"balance"), None) => {
            json_response(
                json!({"confirmed": null, "unconfirmed": null}),
                TTL_MEMPOOL_RECENT
            )
        }

        (&Method::GET, Some(&"blockchain"), Some(&"scripthash"), Some(script_str), Some(&"history"), None) => {
            let script_hash = Sha256dHash::from_hex(script_str).chain_err(|| "bad script_hash")?;
            let status = query.status(&script_hash[..])?;
            json_response(
                json!(serde_json::Value::Array(
                    status
                        .history()
                        .into_iter()
                        .map(|item| json!({"tx_hash": item.to_hex()}))
                        .collect()
                )),
                TTL_MEMPOOL_RECENT
            )
        }

        (&Method::GET, Some(&"blockchain"), Some(&"scripthashes"), Some(&"history"), None, None) => {
            let script_hashes = query_params
                .get("scripthashes")
                .cloned()
                .ok_or_else(|| HttpError::from("Missing scripthashes".to_string()))?;

            let v_script_hashes: Vec<&str> = script_hashes.split(',').collect();

            let mut v_txids = vec![];

            for script_str in v_script_hashes.iter() {
                let script_hash = Sha256dHash::from_hex(script_str).chain_err(|| "bad script_hash")?;
                let status = query.status(&script_hash[..])?;
                let txids: Vec<String> = status
                    .history()
                    .into_iter()
                    .map(|item| item.to_hex())
                    .collect();
                v_txids.push(json!({
                    "script_hash": script_str,
                    "txids": txids}));
            }

            json_response(
                json!(serde_json::Value::Array(v_txids)),
                TTL_MEMPOOL_RECENT
            )
        }

        (&Method::GET, Some(&"blocks"), Some(&"tip"), None, None, None) => {
            let entry = query.get_best_header()?;
            let hex_header = hex::encode(serialize(entry.header()));
            json_response(
                json!({"hex": hex_header, "height": entry.height()}),
                TTL_SHORT
            )
        }

        _ => Err(HttpError::not_found(format!(
          "endpoint does not exist {:?}",
          uri.path()
        ))),
    }
}


#[derive(Debug)]
struct HttpError(StatusCode, String);

impl HttpError {
    fn not_found(msg: String) -> Self {
        HttpError(StatusCode::NOT_FOUND, msg)
    }
}

impl From<String> for HttpError {
    fn from(msg: String) -> Self {
        HttpError(StatusCode::BAD_REQUEST, msg)
    }
}

impl From<ParseIntError> for HttpError {
    fn from(_e: ParseIntError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid number".to_string())
    }
}

impl From<HashError> for HttpError {
    fn from(_e: HashError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hash string".to_string())
    }
}

impl From<FromHexError> for HttpError {
    fn from(_e: FromHexError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hex string".to_string())
    }
}

impl From<bitcoin::hashes::hex::Error> for HttpError {
    fn from(_e: bitcoin::hashes::hex::Error) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hex string".to_string())
    }
}

impl From<bitcoin::util::address::Error> for HttpError {
    fn from(_e: bitcoin::util::address::Error) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid Bitcoin address".to_string())
    }
}

impl From<errors::Error> for HttpError {
    fn from(e: errors::Error) -> Self {
        warn!("errors::Error: {:?}", e);
        match e.description().to_string().as_ref() {
            "getblock RPC error: {\"code\":-5,\"message\":\"Block not found\"}" => {
              HttpError::not_found("Block not found".to_string())
            }
            _ => HttpError::from(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for HttpError {
    fn from(e: serde_json::Error) -> Self {
        HttpError::from(e.to_string())
    }
}

impl From<encode::Error> for HttpError {
    fn from(e: encode::Error) -> Self {
       HttpError::from(e.to_string())
    }
}

impl From<std::string::FromUtf8Error> for HttpError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        HttpError::from(e.to_string())
    }
}
