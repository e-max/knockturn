use crate::clients::PlainHttpAuth;
use crate::errors::Error;
use actix::{Actor, Addr};
use actix_web::client::{self, ClientConnector};
use actix_web::HttpMessage;
use futures::Future;
use log::{debug, error};
use serde::Deserialize;
use serde_json::from_slice;
use std::str::from_utf8;
use std::time::Duration;

const CHAIN_OUTPUTS_BY_HEIGHT: &'static str = "v1/chain/outputs/byheight";

#[derive(Clone)]
pub struct Node {
    conn: Addr<ClientConnector>,
    username: String,
    password: String,
    url: String,
}

impl Node {
    pub fn new(url: &str, username: &str, password: &str) -> Self {
        let connector = ClientConnector::default()
            .conn_lifetime(Duration::from_secs(300))
            .conn_keep_alive(Duration::from_secs(300));
        Node {
            url: url.trim_end_matches('/').to_owned(),
            username: username.to_owned(),
            password: password.to_owned(),
            conn: connector.start(),
        }
    }

    pub fn blocks(&self, start: i64, end: i64) -> impl Future<Item = Vec<Block>, Error = Error> {
        let url = format!(
            "{}/{}?start_height={}&end_height={}",
            self.url, CHAIN_OUTPUTS_BY_HEIGHT, start, end
        );
        debug!("Get latest blocks from node {}", url);
        client::get(&url) // <- Create request builder
            .auth(&self.username, &self.password)
            .finish()
            .unwrap()
            .send() // <- Send http request
            .map_err(|e| Error::NodeAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::NodeAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(resp)
                }
            })
            .and_then(|resp| {
                // <- server http response
                debug!("Response: {:?}", resp);
                resp.body()
                    .map_err(|e| Error::NodeAPIError(s!(e)))
                    .and_then(move |bytes| {
                        let blocks: Vec<Block> = from_slice(&bytes).map_err(|e| {
                            error!(
                                "Cannot decode json {:?}:\n with error {} ",
                                from_utf8(&bytes),
                                e
                            );
                            Error::NodeAPIError(format!("Cannot decode json {}", e))
                        })?;
                        Ok(blocks)
                    })
            })
    }
}

#[derive(Deserialize, Debug)]
pub struct Block {
    pub header: Header,
    pub outputs: Vec<Output>,
}

#[derive(Deserialize, Debug)]
pub struct Header {
    pub height: u64,
}

#[derive(Deserialize, Debug)]
pub struct Output {
    pub output_type: String,
    pub commit: String,
    pub block_height: u64,
}

impl Output {
    pub fn is_coinbase(&self) -> bool {
        self.output_type == "Coinbase"
    }
}

const sample: &'static str = r#"
 {
    "header": {
      "hash": "077360fcf848b71c8c07bb35fd361ad8aa5b9608cd62130a1b5a50d8c071f091",
      "height": 84586,
      "previous": "1d4ab68f8d7ae4e32116b0a84a2046a1f737f1f8431aa7b934248b4a58a0b14c"
    },
    "outputs": [
      {
        "output_type": "Coinbase",
        "commit": "0808d9594ac88429049238fcce5bf69944e4c05f87ad318c56b776076de630d46c",
        "spent": false,
        "proof": null,
        "proof_hash": "16519ea0616790dbe3c7acc6f3e40aa0e776106cad02e20f94a003a646d55bd9",
        "block_height": 84586,
        "merkle_proof": "0000000000044ddc000000000000000a60d7b823fd9344019c69c29b4b027818e4722e1454e716c88ddd999207c7f22ca60c549e1a0e5f10bf9048c222031c90a72ac0916bc40550034268afaddf255fce7a670028038f8fb647a8d4c373254d0b0e6b0fb9dfcff4e43dda86b294833f1cdebaec85c75b0c630528d8360689893f2557fe47dbf5c667ebb5f2647803269a38194d07acd484e93e847fe5292c09c2a7499195357c65f64bf09374e85ef4fea77649aa53801ccd7b7ccff258aa08f9b16951a2098c6539bea0269ea72b0d2c0d669579504dbd1788ee804e487cf86898199cb3a881467bec682fa7e38c9b5bc2db1dff1d1b3f0851a4323063aca898c251f8f3ab6b1ce8dd7f2a4279cf341f3c2c3e9f6b23a2b49349964a959a8a4e33e1549194d721add42eba8c4a134758661f2b62fa23eb399def5cec17297cf5d612d3186deb30f598ae78d874dc13",
        "mmr_index": 282073
      },
      {
        "output_type": "Transaction",
        "commit": "08411bb50f21a1d154687cda50410473ce0fe6bc723c6c76dae5a28eea4a189ea6",
        "spent": false,
        "proof": null,
        "proof_hash": "f1768ef908bae2d170e57b0ca064a40647eeb5e8b07e4fd0cd74ce8ba205378f",
        "block_height": 84586,
        "merkle_proof": null,
        "mmr_index": 282074
      },
            {
        "output_type": "Transaction",
        "commit": "093ae209d95233eefee1a97ce27126cd2f3f41e17662e6f79386729ba07125cd7c",
        "spent": false,
        "proof": null,
        "proof_hash": "6406260bbe91ff428f94f5bbc9f9da2b8942f7305c822430a6a034265329b8c8",
        "block_height": 84586,
        "merkle_proof": null,
        "mmr_index": 282076
      }
    ]
  }"#;
