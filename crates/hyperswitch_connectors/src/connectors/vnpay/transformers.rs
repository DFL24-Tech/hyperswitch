use std::collections::BTreeMap;

use common_enums::{AttemptStatus, Currency};
use common_utils::{crypto, errors::CustomResult};
use error_stack::{report, ResultExt};
use hyperswitch_domain_models::{
    router_data::{ConnectorAuthType, ErrorResponse},
    router_flow_types::{Authorize, PSync},
    router_request_types::{PaymentsAuthorizeData, PaymentsSyncData, ResponseId},
    router_response_types::{PaymentsResponseData, RedirectForm},
    types::{PaymentsAuthorizeRouterData, PaymentsSyncRouterData},
};
use hyperswitch_interfaces::errors::ConnectorError;
use hyperswitch_masking::{ExposeInterface, Secret};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

type Error = error_stack::Report<ConnectorError>;

// ─── Auth ─────────────────────────────────────────────────────────────────────

pub struct VnpayAuthType {
    /// HMAC-SHA512 signing secret (vnp_HashSecret)
    pub(super) hash_secret: Secret<String>,
    /// Merchant terminal code (vnp_TmnCode)
    pub(super) tmn_code: Secret<String>,
}

impl TryFrom<&ConnectorAuthType> for VnpayAuthType {
    type Error = Error;
    fn try_from(auth_type: &ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            ConnectorAuthType::BodyKey { api_key, key1 } => Ok(Self {
                hash_secret: api_key.to_owned(),
                tmn_code: key1.to_owned(),
            }),
            _ => Err(ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}

// ─── RouterData wrapper ───────────────────────────────────────────────────────

pub struct VnpayRouterData<T> {
    pub amount: i64, // VND amount in minor units
    pub router_data: T,
}

impl<T> From<(i64, T)> for VnpayRouterData<T> {
    fn from((amount, router_data): (i64, T)) -> Self {
        Self {
            amount,
            router_data,
        }
    }
}

// ─── Authorize (redirect URL builder) ────────────────────────────────────────

pub struct VnpayPaymentParams {
    pub tmn_code: String,
    pub amount_x100: i64, // VNPay requires amount × 100
    pub txn_ref: String,
    pub order_info: String,
    pub return_url: String,
    pub ip_addr: String,
    pub create_date: String,
    pub locale: String,
}

impl VnpayPaymentParams {
    /// Build a BTreeMap of params (sorted by key) excluding the signature.
    pub fn to_sorted_map(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("vnp_Amount".to_string(), self.amount_x100.to_string());
        map.insert("vnp_Command".to_string(), "pay".to_string());
        map.insert("vnp_CreateDate".to_string(), self.create_date.clone());
        map.insert("vnp_CurrCode".to_string(), "VND".to_string());
        map.insert("vnp_IpAddr".to_string(), self.ip_addr.clone());
        map.insert("vnp_Locale".to_string(), self.locale.clone());
        map.insert("vnp_OrderInfo".to_string(), self.order_info.clone());
        map.insert("vnp_OrderType".to_string(), "other".to_string());
        map.insert("vnp_ReturnUrl".to_string(), self.return_url.clone());
        map.insert("vnp_TmnCode".to_string(), self.tmn_code.clone());
        map.insert("vnp_TxnRef".to_string(), self.txn_ref.clone());
        map.insert("vnp_Version".to_string(), "2.1.0".to_string());
        map
    }
}

impl TryFrom<&VnpayRouterData<&PaymentsAuthorizeRouterData>> for VnpayPaymentParams {
    type Error = Error;
    fn try_from(item: &VnpayRouterData<&PaymentsAuthorizeRouterData>) -> Result<Self, Self::Error> {
        let auth = VnpayAuthType::try_from(&item.router_data.connector_auth_type)?;

        if item.router_data.request.currency != Currency::VND {
            return Err(report!(ConnectorError::NotSupported {
                message: "VNPay only supports VND".to_string(),
                connector: "vnpay",
            }));
        }

        // VNPay expects amount × 100 (e.g. 50,000 VND → 5,000,000)
        let amount_x100 = item.amount * 100;

        let txn_ref = item.router_data.connector_request_reference_id.clone();

        let order_info = item
            .router_data
            .request
            .statement_descriptor
            .clone()
            .unwrap_or_else(|| format!("Thanh toan don hang {}", txn_ref));

        let return_url = item
            .router_data
            .request
            .complete_authorize_url
            .clone()
            .ok_or(ConnectorError::MissingRequiredField {
                field_name: "complete_authorize_url",
            })?;

        let ip_addr = item
            .router_data
            .request
            .browser_info
            .as_ref()
            .and_then(|b| b.ip_address)
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        Ok(Self {
            tmn_code: auth.tmn_code.expose(),
            amount_x100,
            txn_ref,
            order_info,
            return_url,
            ip_addr,
            create_date: format_vnpay_datetime(OffsetDateTime::now_utc()),
            locale: "vn".to_string(),
        })
    }
}

// ─── PSync request ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct VnpayQueryRequest {
    #[serde(rename = "vnp_RequestId")]
    pub request_id: String,
    #[serde(rename = "vnp_Version")]
    pub version: String,
    #[serde(rename = "vnp_Command")]
    pub command: String,
    #[serde(rename = "vnp_TmnCode")]
    pub tmn_code: String,
    #[serde(rename = "vnp_TxnRef")]
    pub txn_ref: String,
    #[serde(rename = "vnp_OrderInfo")]
    pub order_info: String,
    #[serde(rename = "vnp_TransDate")]
    pub trans_date: String,
    #[serde(rename = "vnp_CreateDate")]
    pub create_date: String,
    #[serde(rename = "vnp_IpAddr")]
    pub ip_addr: String,
    #[serde(rename = "vnp_SecureHash")]
    pub secure_hash: String,
}

impl TryFrom<&PaymentsSyncRouterData> for VnpayQueryRequest {
    type Error = Error;
    fn try_from(data: &PaymentsSyncRouterData) -> Result<Self, Self::Error> {
        let auth = VnpayAuthType::try_from(&data.connector_auth_type)?;

        let txn_ref = data
            .request
            .connector_transaction_id
            .get_connector_transaction_id()
            .change_context(ConnectorError::MissingConnectorTransactionID)?;

        let create_date = format_vnpay_datetime(OffsetDateTime::now_utc());

        let mut params = BTreeMap::new();
        params.insert("vnp_Command".to_string(), "querydr".to_string());
        params.insert("vnp_CreateDate".to_string(), create_date.clone());
        params.insert("vnp_IpAddr".to_string(), "127.0.0.1".to_string());
        params.insert(
            "vnp_OrderInfo".to_string(),
            format!("Kiem tra GD {}", txn_ref),
        );
        params.insert(
            "vnp_RequestId".to_string(),
            uuid::Uuid::new_v4().to_string(),
        );
        params.insert("vnp_TmnCode".to_string(), auth.tmn_code.peek().to_string());
        params.insert("vnp_TransDate".to_string(), create_date.clone());
        params.insert("vnp_TxnRef".to_string(), txn_ref.clone());
        params.insert("vnp_Version".to_string(), "2.1.0".to_string());

        let secure_hash = compute_vnpay_signature(&params, auth.hash_secret.peek())?;

        Ok(Self {
            request_id: params["vnp_RequestId"].clone(),
            version: "2.1.0".to_string(),
            command: "querydr".to_string(),
            tmn_code: auth.tmn_code.expose(),
            txn_ref,
            order_info: params["vnp_OrderInfo"].clone(),
            trans_date: create_date.clone(),
            create_date,
            ip_addr: "127.0.0.1".to_string(),
            secure_hash,
        })
    }
}

// ─── PSync response ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct VnpayQueryResponse {
    #[serde(rename = "vnp_ResponseCode")]
    pub response_code: String,
    #[serde(rename = "vnp_Message")]
    pub message: Option<String>,
    #[serde(rename = "vnp_TxnRef")]
    pub txn_ref: Option<String>,
    #[serde(rename = "vnp_TransactionNo")]
    pub transaction_no: Option<String>,
    #[serde(rename = "vnp_TransactionStatus")]
    pub transaction_status: Option<String>,
}

impl VnpayQueryResponse {
    pub fn to_attempt_status(&self) -> AttemptStatus {
        match self.response_code.as_str() {
            "00" => AttemptStatus::Charged,
            "07" => AttemptStatus::Pending,
            _ => AttemptStatus::Failure,
        }
    }
}

// ─── IPN / Return URL response ────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct VnpayIpnResponse {
    #[serde(rename = "vnp_TmnCode")]
    pub tmn_code: String,
    #[serde(rename = "vnp_Amount")]
    pub amount: String,
    #[serde(rename = "vnp_BankCode")]
    pub bank_code: Option<String>,
    #[serde(rename = "vnp_BankTranNo")]
    pub bank_tran_no: Option<String>,
    #[serde(rename = "vnp_CardType")]
    pub card_type: Option<String>,
    #[serde(rename = "vnp_OrderInfo")]
    pub order_info: String,
    #[serde(rename = "vnp_TransactionNo")]
    pub transaction_no: String,
    #[serde(rename = "vnp_ResponseCode")]
    pub response_code: String,
    #[serde(rename = "vnp_TransactionStatus")]
    pub transaction_status: String,
    #[serde(rename = "vnp_TxnRef")]
    pub txn_ref: String,
    #[serde(rename = "vnp_SecureHash")]
    pub secure_hash: String,
}

impl VnpayIpnResponse {
    pub fn to_attempt_status(&self) -> AttemptStatus {
        match self.response_code.as_str() {
            "00" => AttemptStatus::Charged,
            "07" => AttemptStatus::Pending,
            "24" => AttemptStatus::Voided,
            _ => AttemptStatus::Failure,
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// HMAC-SHA512 over URL-encoded, alphabetically-sorted params.
pub fn compute_vnpay_signature(
    params: &BTreeMap<String, String>,
    secret: &str,
) -> CustomResult<String, ConnectorError> {
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let sig_bytes =
        crypto::HmacSha512::sign_message(&crypto::HmacSha512, secret.as_bytes(), query.as_bytes())
            .change_context(ConnectorError::RequestEncodingFailed)?;

    Ok(hex::encode(sig_bytes))
}

/// Format a timestamp as `YYYYMMDDHHmmss` in UTC+7 (Vietnam time).
pub fn format_vnpay_datetime(dt: OffsetDateTime) -> String {
    let vn_offset = time::UtcOffset::from_hms(7, 0, 0).unwrap_or(time::UtcOffset::UTC);
    let vn_dt = dt.to_offset(vn_offset);
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        vn_dt.year(),
        vn_dt.month() as u8,
        vn_dt.day(),
        vn_dt.hour(),
        vn_dt.minute(),
        vn_dt.second(),
    )
}

/// Build a signed VNPay redirect URL from sorted params + base URL.
pub fn build_redirect_url(
    base_url: &str,
    params: &VnpayPaymentParams,
    hash_secret: &str,
) -> CustomResult<String, ConnectorError> {
    let mut map = params.to_sorted_map();
    let sig = compute_vnpay_signature(&map, hash_secret)?;
    map.insert("vnp_SecureHash".to_string(), sig);

    let query = map
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    Ok(format!("{}paymentv2/vpcpay.html?{}", base_url, query))
}
