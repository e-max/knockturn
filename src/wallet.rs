use crate::errors::Error;
use crate::jsonrpc;
use crate::ser;
use actix_web::client::Client;
use chrono::{DateTime, Utc};
use futures::future::{err, ok, Either};
use futures::Future;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use serde_json;
use serde_json::from_slice;
use std::iter::Iterator;
use std::str::from_utf8;
use uuid::Uuid;

#[derive(Clone)]
pub struct Wallet {
    //    client: Client,
    username: String,
    password: String,
    url: String,
}

const RETRIEVE_TXS_URL: &'static str = "v1/wallet/owner/retrieve_txs";
const RECEIVE_URL: &'static str = "v1/wallet/foreign/receive_tx";
const SEND_URL: &'static str = "/v1/wallet/owner/issue_send_tx";
const FINALIZE_URL: &'static str = "/v1/wallet/owner/finalize_tx";
const CANCEL_TX_URL: &'static str = "/v1/wallet/owner/cancel_tx";
const POST_TX_URL: &'static str = "/v1/wallet/owner/post_tx?fluff";
const JSONRPC_FOREIGN_URL: &'static str = "v2/foreign";
const JSONRPC_OWNER_URL: &'static str = "v2/owner";

/// V2 Init / Send TX API Args
#[derive(Clone, Serialize, Deserialize)]
pub struct InitTxArgs {
    /// The human readable account name from which to draw outputs
    /// for the transaction, overriding whatever the active account is as set via the
    /// [`set_active_account`](../grin_wallet_api/owner/struct.Owner.html#method.set_active_account) method.
    pub src_acct_name: Option<String>,
    #[serde(with = "ser::string_or_u64")]
    /// The amount to send, in nanogrins. (`1 G = 1_000_000_000nG`)
    pub amount: u64,
    #[serde(with = "ser::string_or_u64")]
    /// The minimum number of confirmations an output
    /// should have in order to be included in the transaction.
    pub minimum_confirmations: u64,
    /// By default, the wallet selects as many inputs as possible in a
    /// transaction, to reduce the Output set and the fees. The wallet will attempt to spend
    /// include up to `max_outputs` in a transaction, however if this is not enough to cover
    /// the whole amount, the wallet will include more outputs. This parameter should be considered
    /// a soft limit.
    pub max_outputs: u32,
    /// The target number of change outputs to create in the transaction.
    /// The actual number created will be `num_change_outputs` + whatever remainder is needed.
    pub num_change_outputs: u32,
    /// If `true`, attempt to use up as many outputs as
    /// possible to create the transaction, up the 'soft limit' of `max_outputs`. This helps
    /// to reduce the size of the UTXO set and the amount of data stored in the wallet, and
    /// minimizes fees. This will generally result in many inputs and a large change output(s),
    /// usually much larger than the amount being sent. If `false`, the transaction will include
    /// as many outputs as are needed to meet the amount, (and no more) starting with the smallest
    /// value outputs.
    pub selection_strategy_is_use_all: bool,
    /// An optional participant message to include alongside the sender's public
    /// ParticipantData within the slate. This message will include a signature created with the
    /// sender's private excess value, and will be publically verifiable. Note this message is for
    /// the convenience of the participants during the exchange; it is not included in the final
    /// transaction sent to the chain. The message will be truncated to 256 characters.
    pub message: Option<String>,
    /// Optionally set the output target slate version (acceptable
    /// down to the minimum slate version compatible with the current. If `None` the slate
    /// is generated with the latest version.
    pub target_slate_version: Option<u16>,
    /// If true, just return an estimate of the resulting slate, containing fees and amounts
    /// locked without actually locking outputs or creating the transaction. Note if this is set to
    /// 'true', the amount field in the slate will contain the total amount locked, not the provided
    /// transaction amount
    pub estimate_only: Option<bool>,
    /// Sender arguments. If present, the underlying function will also attempt to send the
    /// transaction to a destination and optionally finalize the result
    pub send_args: Option<()>,
}

impl Wallet {
    pub fn new(url: &str, username: &str, password: &str) -> Self {
        //let connector = Connector::new()
        //.conn_lifetime(Duration::from_secs(300))
        //.conn_keep_alive(Duration::from_secs(300))
        //.finish();
        Wallet {
            url: url.trim_end_matches('/').to_owned(),
            username: username.to_owned(),
            password: password.to_owned(),
            //client: Client::build().connector(connector).finish(),
        }
    }

    fn client(&self) -> Client {
        Client::new()
    }

    pub fn jsonrpc_request(
        &self,
        req: jsonrpc::Request,
        owner: bool,
    ) -> impl Future<Item = jsonrpc::Response, Error = Error> {
        let url: String;

        if (owner) {
            url = format!("{}/{}", self.url, JSONRPC_OWNER_URL);
        } else {
            url = format!("{}/{}", self.url, JSONRPC_FOREIGN_URL);
        }

        debug!("Send raw jsonrpc request {}", url);
        self.client()
            .post(&url) // <- Create request builder
            .basic_auth(&self.username, Some(&self.password))
            .send_json(&req)
            .map_err(|e| Error::WalletAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::WalletAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(resp)
                }
            })
            .and_then(|mut resp| {
                // <- server http response
                debug!("Response: {:?}", resp);
                resp.body()
                    .map_err(|e| Error::WalletAPIError(s!(e)))
                    .and_then(move |bytes| {
                        let resp: jsonrpc::Response = from_slice(&bytes).map_err(|e| {
                            error!(
                                "Cannot decode json {:?}:\n with error {} ",
                                from_utf8(&bytes),
                                e
                            );
                            Error::WalletAPIError(format!("Cannot decode json {}", e))
                        })?;
                        Ok(resp)
                    })
            })
    }

    pub fn get_tx(&self, tx_id: &str) -> impl Future<Item = TxLogEntry, Error = Error> {
        debug!("Get transaction from wallet");
        let tx_id = tx_id.to_owned();
        let req = jsonrpc::Request::new(
            "retrieve_txs",
            vec![
                serde_json::to_value(true).unwrap(),
                serde_json::Value::Null,
                serde_json::to_value(&tx_id).unwrap(),
            ],
        );
        self.jsonrpc_request(req, true).and_then(move |mut resp| {
            debug!("Response: {:?}", resp);
            let res: Result<(bool, Vec<TxLogEntry>), String> =
                serde_json::from_value(resp.result.clone()).map_err(|e| {
                    error!("Cannot decode json {:?}:\n with error {} ", resp.result, e);
                    Error::WalletAPIError(format!("Cannot decode json {}", e))
                })?;

            let (updated, txs) = res.map_err(|e| Error::General(s!(e)))?;
            if txs.len() == 0 {
                return Err(Error::WalletAPIError(format!(
                    "Transaction with slate_id {} not found",
                    tx_id
                )));
            }
            if txs.len() > 1 {
                return Err(Error::WalletAPIError(format!(
                    "Wallet returned more than one transaction with slate_id {}",
                    tx_id
                )));
            }
            let tx = txs.into_iter().next().unwrap();
            Ok(tx)
        })
    }

    pub fn receive(&self, slate: &Slate) -> impl Future<Item = Slate, Error = Error> {
        let url = format!("{}/{}", self.url, RECEIVE_URL);
        debug!("Receive slate by wallet  {}", url);
        self.client()
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send_json(slate)
            .map_err(|e| Error::WalletAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::WalletAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(resp)
                }
            })
            .and_then(|mut resp| {
                debug!("Response: {:?}", resp);
                resp.body()
                    .map_err(|e| Error::WalletAPIError(s!(e)))
                    .and_then(move |bytes| {
                        let slate_resp: Slate = from_slice(&bytes).map_err(|e| {
                            error!(
                                "Cannot decode json {:?}:\n with error {} ",
                                from_utf8(&bytes),
                                e
                            );
                            Error::WalletAPIError(format!("Cannot decode json {}", e))
                        })?;
                        Ok(slate_resp)
                    })
            })
    }

    pub fn finalize(&self, slate: &Slate) -> impl Future<Item = Slate, Error = Error> {
        let url = format!("{}/{}", self.url, FINALIZE_URL);
        debug!("Finalize slate by wallet {}", url);
        self.client()
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send_json(slate)
            .map_err(|e| Error::WalletAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::WalletAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(resp)
                }
            })
            .and_then(|mut resp| {
                debug!("Response: {:?}", resp);
                resp.body()
                    .map_err(|e| Error::WalletAPIError(s!(e)))
                    .and_then(move |bytes| {
                        let slate_resp: Slate = from_slice(&bytes).map_err(|e| {
                            error!(
                                "Cannot decode json {:?}:\n with error {} ",
                                from_utf8(&bytes),
                                e
                            );
                            Error::WalletAPIError(format!("Cannot decode json {}", e))
                        })?;
                        Ok(slate_resp)
                    })
            })
    }
    pub fn cancel_tx(&self, tx_slate_id: &str) -> impl Future<Item = (), Error = Error> {
        let url = format!("{}/{}?tx_id={}", self.url, CANCEL_TX_URL, tx_slate_id);
        debug!("Cancel transaction in wallet {}", url);
        self.client()
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| Error::WalletAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::WalletAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(())
                }
            })
    }

    pub fn post_tx(&self) -> impl Future<Item = (), Error = Error> {
        let url = format!("{}/{}", self.url, POST_TX_URL);
        debug!("Post transaction in chain by wallet as {}", url);
        self.client()
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| Error::WalletAPIError(s!(e)))
            .and_then(|resp| {
                if !resp.status().is_success() {
                    Err(Error::WalletAPIError(format!("Error status: {:?}", resp)))
                } else {
                    Ok(())
                }
            })
    }

    pub fn create_slate(
        &self,
        amount: u64,
        message: String,
    ) -> impl Future<Item = jsonrpc::TypedResponse<Slate>, Error = Error> {
        let url = format!("{}/{}", self.url, SEND_URL);
        info!("Try to create payout slate");
        debug!("Receive as {} {}: {}", self.username, self.password, url);
        let payment = InitTxArgs {
            src_acct_name: None,
            amount,
            minimum_confirmations: 10,
            max_outputs: 10,
            num_change_outputs: 1,
            selection_strategy_is_use_all: false,
            message: Some(message),
            target_slate_version: None,
            estimate_only: None,
            send_args: None,
        };

        let req =
            jsonrpc::Request::new("init_send_tx", vec![serde_json::to_value(payment).unwrap()]);

        let newself = self.clone();
        self.jsonrpc_request(req, true)
            .map_err(|e| Error::WalletAPIError(format!("Cannot create slate in wallet: {}", e)))
            .and_then(move |resp| {
                if let Some(err) = resp.error.as_ref() {
                    error!("Cannot create slate in wallet: {}", err);
                    Either::A(ok(jsonrpc::TypedResponse::new(resp)))
                } else {
                    Either::B({
                        match serde_json::from_value::<Result<serde_json::Value, String>>(
                            resp.result.clone(),
                        ) {
                            Ok(val) => {
                                let req = jsonrpc::Request::new(
                                    "tx_lock_outputs",
                                    vec![val.unwrap(), serde_json::json!(0)],
                                );
                                Either::A(
                                    newself
                                        .jsonrpc_request(req, true)
                                        .and_then(move |lock_resp| {
                                            if let Some(err) = lock_resp.error {
                                                error!("Cannot lock outputs in wallet: {}", err);
                                                return Err(Error::WalletAPIError(format!(
                                                    "Cannon lock outputs in wallet: {}",
                                                    err
                                                )));
                                            }
                                            Ok(jsonrpc::TypedResponse::new(resp))
                                        })
                                        .map_err(|e| {
                                            Error::WalletAPIError(format!(
                                                "Cannon lock outputs in wallet: {}",
                                                e
                                            ))
                                        }),
                                )
                            }
                            Err(e) => Either::B(err(Error::WalletAPIError(format!(
                                "Cannon parse slate: {}",
                                e
                            )))),
                        }
                    })
                }
            })
    }
}
/// Optional transaction information, recorded when an event happens
/// to add or remove funds from a wallet. One Transaction log entry
/// maps to one or many outputs
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxLogEntry {
    /// Local id for this transaction (distinct from a slate transaction id)
    pub id: u32,
    /// Slate transaction this entry is associated with, if any
    pub tx_slate_id: Option<String>,
    /// Fee
    #[serde(with = "ser::opt_string_or_u64")]
    pub fee: Option<u64>,
    /// Message data, stored as json
    pub messages: Option<ParticipantMessages>,
}

/// Helper just to facilitate serialization
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParticipantMessages {
    /// included messages
    pub messages: Vec<ParticipantMessageData>,
}

/// Public message data (for serialising and storage)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParticipantMessageData {
    /// id of the particpant in the tx
    #[serde(with = "ser::string_or_u64")]
    pub id: u64,
    /// Public key
    pub public_key: String,
    /// Message,
    pub message: Option<String>,
    /// Signature
    pub message_sig: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParticipantData {
    /// Id of participant in the transaction. (For now, 0=sender, 1=rec)
    pub id: u64,
    /// Public key corresponding to private blinding factor
    pub public_blind_excess: Vec<u8>,
    /// Public key corresponding to private nonce
    pub public_nonce: Vec<u8>,
    /// Public partial signature
    pub part_sig: Option<Vec<u8>>,
    /// A message for other participants
    pub message: Option<String>,
    /// Signature, created with private key corresponding to 'public_blind_excess'
    pub message_sig: Option<Vec<u8>>,
}

/// A 'Slate' is passed around to all parties to build up all of the public
/// transaction data needed to create a finalized transaction. Callers can pass
/// the slate around by whatever means they choose, (but we can provide some
/// binary or JSON serialization helpers here).

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Slate {
    /// Unique transaction ID, selected by sender
    pub id: Uuid,
    /// The core transaction data:
    /// inputs, outputs, kernels, kernel offset
    pub tx: Transaction,
    /// base amount (excluding fee)
    #[serde(with = "ser::string_or_u64")]
    pub amount: u64,
}

/// A transaction
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub body: TransactionBody,
}

impl Transaction {
    pub fn output_commitments(&self) -> Vec<Vec<u8>> {
        self.body.outputs.iter().map(|o| o.commit.clone()).collect()
    }
}

/// TransactionBody is a common abstraction for transaction and block
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionBody {
    /// List of outputs the transaction produces.
    pub outputs: Vec<Output>,
}

/// Output for a transaction, defining the new ownership of coins that are being
/// transferred. The commitment is a blinded value for the output while the
/// range proof guarantees the commitment includes a positive value without
/// overflow and the ownership of the private key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    #[serde(
        serialize_with = "ser::as_hex",
        deserialize_with = "ser::commitment_from_hex"
    )]
    pub commit: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct SendTx {
    amount: u64,
    minimum_confirmations: u64,
    method: &'static str,
    dest: &'static str,
    max_outputs: u8,
    num_change_outputs: u8,
    selection_strategy_is_use_all: bool,
    message: Option<String>,
}

#[cfg(test)]
mod tests {

    #[test]
    fn wallet_get_tx_test() {
        assert!(true);
    }
    #[test]
    fn txs_read_test() {}
}
