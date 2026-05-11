use common_enums::{AttemptStatus, RefundStatus};
use common_utils::{
    crypto::{self, SignMessage},
    errors::CustomResult,
    types::MinorUnit,
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    router_data::{ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::{Authorize, Execute, PSync, RSync},
    router_request_types::{PaymentsAuthorizeData, RefundsData, ResponseId},
    router_response_types::{PaymentsResponseData, RedirectForm, RefundsResponseData},
    types::{PaymentsAuthorizeRouterData, PaymentsSyncRouterData, RefundSyncRouterData, RefundsRouterData},
};
use hyperswitch_interfaces::errors::ConnectorError;
use hyperswitch_masking::{ExposeInterface, PeekInterface, Secret};
use serde::{Deserialize, Serialize};
use common_utils::request::Method;

use crate::{
    types::{RefundsResponseRouterData, ResponseRouterData},
    utils::PaymentsAuthorizeRequestData,
};

type Error = error_stack::Report<ConnectorError>;

pub struct MomoRouterData<T> {
    pub amount: MinorUnit,
    pub router_data: T,
}

impl<T> From<(MinorUnit, T)> for MomoRouterData<T> {
    fn from((amount, router_data): (MinorUnit, T)) -> Self {
        Self {
            amount,
            router_data,
        }
    }
}

pub struct MomoAuthType {
    pub(super) partner_code: Secret<String>,
    pub(super) access_key: Secret<String>,
    pub(super) secret_key: Secret<String>,
}

impl TryFrom<&ConnectorAuthType> for MomoAuthType {
    type Error = Error;
    fn try_from(auth_type: &ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            ConnectorAuthType::SignatureKey {
                api_key,
                key1,
                api_secret,
            } => Ok(Self {
                partner_code: api_key.clone(),
                access_key: key1.clone(),
                secret_key: api_secret.clone(),
            }),
            _ => Err(ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}

fn compute_signature(message: &str, secret_key: &Secret<String>) -> CustomResult<String, ConnectorError> {
    let bytes = crypto::HmacSha256
        .sign_message(secret_key.peek().as_bytes(), message.as_bytes())
        .change_context(ConnectorError::RequestEncodingFailed)?;
    Ok(hex::encode(bytes))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MomoRequestType {
    #[serde(rename = "captureWallet")]
    CaptureWallet,
}

// ─── Create payment ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoCreatePaymentRequest {
    pub partner_code: Secret<String>,
    pub request_type: MomoRequestType,
    pub ipn_url: String,
    pub redirect_url: String,
    pub order_id: String,
    pub amount: i64,
    pub order_info: String,
    pub request_id: String,
    pub extra_data: String,
    pub lang: String,
    pub signature: String,
}

impl TryFrom<&MomoRouterData<&PaymentsAuthorizeRouterData>> for MomoCreatePaymentRequest {
    type Error = Error;
    fn try_from(
        item: &MomoRouterData<&PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        let auth = MomoAuthType::try_from(&item.router_data.connector_auth_type)?;
        let order_id = item.router_data.attempt_id.clone();
        let request_id = order_id.clone();
        let amount = item.amount.get_amount_as_i64();
        let partner_code_str = auth.partner_code.peek().to_string();
        let access_key_str = auth.access_key.peek().to_string();
        let ipn_url = item.router_data.request.get_webhook_url()?;
        let redirect_url = item.router_data.request.get_router_return_url()?;
        let extra_data = String::new();
        let order_info = item
            .router_data
            .request
            .order_details
            .as_ref()
            .and_then(|o| o.first())
            .map(|d| d.product_name.clone())
            .unwrap_or_else(|| "Payment".to_string());

        let message = format!(
            "accessKey={}&amount={}&extraData={}&ipnUrl={}&orderId={}&orderInfo={}&partnerCode={}&redirectUrl={}&requestId={}&requestType=captureWallet",
            access_key_str,
            amount,
            extra_data,
            ipn_url,
            order_id,
            order_info,
            partner_code_str,
            redirect_url,
            request_id,
        );
        let signature = compute_signature(&message, &auth.secret_key)?;

        Ok(Self {
            partner_code: auth.partner_code,
            request_type: MomoRequestType::CaptureWallet,
            ipn_url,
            redirect_url,
            order_id,
            amount,
            order_info,
            request_id,
            extra_data,
            lang: "vi".to_string(),
            signature,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoCreatePaymentResponse {
    pub partner_code: String,
    pub order_id: String,
    pub request_id: String,
    pub amount: i64,
    pub response_time: Option<i64>,
    pub message: String,
    pub result_code: i64,
    pub pay_url: Option<String>,
    pub deeplink: Option<String>,
    pub qr_code_data: Option<String>,
}

impl
    TryFrom<
        ResponseRouterData<
            Authorize,
            MomoCreatePaymentResponse,
            PaymentsAuthorizeData,
            PaymentsResponseData,
        >,
    > for RouterData<Authorize, PaymentsAuthorizeData, PaymentsResponseData>
{
    type Error = Error;
    fn try_from(
        item: ResponseRouterData<
            Authorize,
            MomoCreatePaymentResponse,
            PaymentsAuthorizeData,
            PaymentsResponseData,
        >,
    ) -> Result<Self, Self::Error> {
        let (response, status) = if item.response.result_code == 0 {
            let pay_url = item
                .response
                .pay_url
                .as_deref()
                .ok_or(ConnectorError::MissingRequiredField {
                    field_name: "payUrl",
                })?;
            let parsed_url = url::Url::parse(pay_url).change_context(
                ConnectorError::ResponseDeserializationFailed,
            )?;
            (
                Ok(PaymentsResponseData::TransactionResponse {
                    resource_id: ResponseId::ConnectorTransactionId(
                        item.response.order_id.clone(),
                    ),
                    redirection_data: Box::new(Some(RedirectForm::from((
                        parsed_url,
                        Method::Get,
                    )))),
                    mandate_reference: Box::new(None),
                    connector_metadata: None,
                    network_txn_id: None,
                    connector_response_reference_id: Some(item.response.order_id),
                    incremental_authorization_allowed: None,
                    authentication_data: None,
                    charges: None,
                }),
                AttemptStatus::AuthenticationPending,
            )
        } else {
            (
                Err(ErrorResponse {
                    code: item.response.result_code.to_string(),
                    message: item.response.message.clone(),
                    reason: Some(item.response.message),
                    status_code: item.http_code,
                    attempt_status: None,
                    connector_transaction_id: None,
                    connector_response_reference_id: None,
                    network_advice_code: None,
                    network_decline_code: None,
                    network_error_message: None,
                    connector_metadata: None,
                }),
                AttemptStatus::Failure,
            )
        };
        Ok(Self {
            status,
            response,
            ..item.data
        })
    }
}

// ─── PSync (query) ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoQueryRequest {
    pub partner_code: Secret<String>,
    pub request_id: String,
    pub order_id: String,
    pub lang: String,
    pub signature: String,
}

impl TryFrom<&PaymentsSyncRouterData> for MomoQueryRequest {
    type Error = Error;
    fn try_from(item: &PaymentsSyncRouterData) -> Result<Self, Self::Error> {
        let auth = MomoAuthType::try_from(&item.connector_auth_type)?;
        let order_id = item.attempt_id.clone();
        let request_id = item.attempt_id.clone();
        let partner_code_str = auth.partner_code.peek().to_string();
        let access_key_str = auth.access_key.peek().to_string();

        let message = format!(
            "accessKey={}&orderId={}&partnerCode={}&requestId={}",
            access_key_str, order_id, partner_code_str, request_id,
        );
        let signature = compute_signature(&message, &auth.secret_key)?;

        Ok(Self {
            partner_code: auth.partner_code,
            request_id,
            order_id,
            lang: "vi".to_string(),
            signature,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoQueryResponse {
    pub partner_code: String,
    pub request_id: Option<String>,
    pub order_id: String,
    pub extra_data: Option<String>,
    pub amount: Option<i64>,
    pub trans_id: Option<i64>,
    pub pay_type: Option<String>,
    pub result_code: i64,
    pub message: String,
    pub response_time: Option<i64>,
    pub payment_option: Option<String>,
}

fn momo_result_code_to_attempt_status(result_code: i64) -> AttemptStatus {
    match result_code {
        0 => AttemptStatus::Charged,
        9000 => AttemptStatus::Authorized,
        1000 => AttemptStatus::AuthenticationPending,
        _ => AttemptStatus::Failure,
    }
}

impl<F, T> TryFrom<ResponseRouterData<F, MomoQueryResponse, T, PaymentsResponseData>>
    for RouterData<F, T, PaymentsResponseData>
{
    type Error = Error;
    fn try_from(
        item: ResponseRouterData<F, MomoQueryResponse, T, PaymentsResponseData>,
    ) -> Result<Self, Self::Error> {
        let result_code = item.response.result_code;
        let status = momo_result_code_to_attempt_status(result_code);

        // Store transId in connector_metadata so refunds can access it
        let connector_metadata = item
            .response
            .trans_id
            .filter(|&id| id != 0)
            .map(|trans_id| serde_json::json!({ "trans_id": trans_id }));

        let (response, status) = if result_code == 0 || result_code == 9000 || result_code == 1000 {
            (
                Ok(PaymentsResponseData::TransactionResponse {
                    resource_id: ResponseId::ConnectorTransactionId(item.response.order_id.clone()),
                    redirection_data: Box::new(None),
                    mandate_reference: Box::new(None),
                    connector_metadata,
                    network_txn_id: None,
                    connector_response_reference_id: Some(item.response.order_id),
                    incremental_authorization_allowed: None,
                    authentication_data: None,
                    charges: None,
                }),
                status,
            )
        } else {
            (
                Err(ErrorResponse {
                    code: result_code.to_string(),
                    message: item.response.message.clone(),
                    reason: Some(item.response.message),
                    status_code: item.http_code,
                    attempt_status: None,
                    connector_transaction_id: None,
                    connector_response_reference_id: None,
                    network_advice_code: None,
                    network_decline_code: None,
                    network_error_message: None,
                    connector_metadata: None,
                }),
                AttemptStatus::Failure,
            )
        };
        Ok(Self {
            status,
            response,
            ..item.data
        })
    }
}

// ─── Refund ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoRefundRequest {
    pub partner_code: Secret<String>,
    pub order_id: String,
    pub request_id: String,
    pub amount: i64,
    pub trans_id: i64,
    pub lang: String,
    pub description: String,
    pub signature: String,
}

impl<F> TryFrom<&MomoRouterData<&RefundsRouterData<F>>> for MomoRefundRequest {
    type Error = Error;
    fn try_from(item: &MomoRouterData<&RefundsRouterData<F>>) -> Result<Self, Self::Error> {
        let auth = MomoAuthType::try_from(&item.router_data.connector_auth_type)?;
        let order_id = item.router_data.request.refund_id.clone();
        let request_id = item.router_data.request.refund_id.clone();
        let amount = item.amount.get_amount_as_i64();
        let partner_code_str = auth.partner_code.peek().to_string();
        let access_key_str = auth.access_key.peek().to_string();
        let description = item
            .router_data
            .request
            .reason
            .clone()
            .unwrap_or_default();

        let trans_id: i64 = item
            .router_data
            .request
            .connector_metadata
            .as_ref()
            .and_then(|meta| meta.get("trans_id"))
            .and_then(|v| v.as_i64())
            .ok_or(ConnectorError::MissingRequiredField {
                field_name: "trans_id",
            })?;

        let message = format!(
            "accessKey={}&amount={}&description={}&orderId={}&partnerCode={}&requestId={}&transId={}",
            access_key_str,
            amount,
            description,
            order_id,
            partner_code_str,
            request_id,
            trans_id,
        );
        let signature = compute_signature(&message, &auth.secret_key)?;

        Ok(Self {
            partner_code: auth.partner_code,
            order_id,
            request_id,
            amount,
            trans_id,
            lang: "vi".to_string(),
            description,
            signature,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoRefundResponse {
    pub partner_code: String,
    pub order_id: String,
    pub request_id: String,
    pub amount: Option<i64>,
    pub trans_id: Option<i64>,
    pub result_code: i64,
    pub message: String,
    pub response_time: Option<i64>,
}

impl TryFrom<RefundsResponseRouterData<Execute, MomoRefundResponse>>
    for RefundsRouterData<Execute>
{
    type Error = Error;
    fn try_from(
        item: RefundsResponseRouterData<Execute, MomoRefundResponse>,
    ) -> Result<Self, Self::Error> {
        let refund_status = if item.response.result_code == 0 {
            RefundStatus::Success
        } else {
            RefundStatus::Failure
        };
        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id: item.response.order_id,
                refund_status,
            }),
            ..item.data
        })
    }
}

// ─── RSync (refund query) ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoRefundQueryRequest {
    pub partner_code: Secret<String>,
    pub request_id: String,
    pub order_id: String,
    pub lang: String,
    pub signature: String,
}

impl TryFrom<&RefundSyncRouterData> for MomoRefundQueryRequest {
    type Error = Error;
    fn try_from(item: &RefundSyncRouterData) -> Result<Self, Self::Error> {
        let auth = MomoAuthType::try_from(&item.connector_auth_type)?;
        let order_id = item
            .request
            .connector_refund_id
            .clone()
            .ok_or(ConnectorError::MissingConnectorRefundID)?;
        let request_id = order_id.clone();
        let partner_code_str = auth.partner_code.peek().to_string();
        let access_key_str = auth.access_key.peek().to_string();

        let message = format!(
            "accessKey={}&orderId={}&partnerCode={}&requestId={}",
            access_key_str, order_id, partner_code_str, request_id,
        );
        let signature = compute_signature(&message, &auth.secret_key)?;

        Ok(Self {
            partner_code: auth.partner_code,
            request_id,
            order_id,
            lang: "vi".to_string(),
            signature,
        })
    }
}

impl TryFrom<RefundsResponseRouterData<RSync, MomoQueryResponse>>
    for RefundsRouterData<RSync>
{
    type Error = Error;
    fn try_from(
        item: RefundsResponseRouterData<RSync, MomoQueryResponse>,
    ) -> Result<Self, Self::Error> {
        let refund_status = if item.response.result_code == 0 {
            RefundStatus::Success
        } else if item.response.result_code == 1000 {
            RefundStatus::Pending
        } else {
            RefundStatus::Failure
        };
        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id: item.response.order_id,
                refund_status,
            }),
            ..item.data
        })
    }
}

// ─── IPN / Webhook ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoIpnPayload {
    pub partner_code: String,
    pub order_id: String,
    pub request_id: String,
    pub amount: i64,
    pub order_info: Option<String>,
    pub order_type: Option<String>,
    pub trans_id: i64,
    pub result_code: i64,
    pub message: String,
    pub pay_type: Option<String>,
    pub response_time: i64,
    pub extra_data: Option<String>,
    pub signature: String,
}

pub fn get_momo_webhook_event(result_code: i64) -> api_models::webhooks::IncomingWebhookEvent {
    match result_code {
        0 => api_models::webhooks::IncomingWebhookEvent::PaymentIntentSuccess,
        9000 => api_models::webhooks::IncomingWebhookEvent::PaymentIntentAuthorizationSuccess,
        _ => api_models::webhooks::IncomingWebhookEvent::PaymentIntentFailure,
    }
}

// ─── Error response ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomoErrorResponse {
    pub result_code: Option<i64>,
    pub message: Option<String>,
}
