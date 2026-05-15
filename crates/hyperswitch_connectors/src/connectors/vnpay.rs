pub mod transformers;

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use common_enums::{enums, CallConnectorAction, PaymentAction};
use common_utils::{
    errors::CustomResult,
    ext_traits::BytesExt,
    request::{Method, Request, RequestBuilder, RequestContent},
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    router_data::{AccessToken, ErrorResponse, RouterData},
    router_flow_types::{
        AccessTokenAuth, Authorize, Capture, Execute, PSync, PaymentMethodToken, RSync, Session,
        SetupMandate, Void,
    },
    router_request_types::{
        AccessTokenRequestData, PaymentMethodTokenizationData, PaymentsAuthorizeData,
        PaymentsCancelData, PaymentsCaptureData, PaymentsSessionData, PaymentsSyncData,
        RefundsData, ResponseId, SetupMandateRequestData,
    },
    router_response_types::{
        ConnectorInfo, PaymentMethodDetails, PaymentsResponseData, RedirectForm,
        RefundsResponseData, SupportedPaymentMethods, SupportedPaymentMethodsExt,
    },
    types::{
        PaymentsAuthorizeRouterData, PaymentsSyncRouterData,
    },
};
use hyperswitch_interfaces::{
    api::{
        self, ConnectorCommon, ConnectorCommonExt, ConnectorIntegration, ConnectorRedirectResponse,
        ConnectorSpecifications, ConnectorValidation,
    },
    configs::Connectors,
    errors::ConnectorError,
    events::connector_api_logs::ConnectorEvent,
    types::{
        PaymentsSyncType, Response,
    },
    webhooks::{IncomingWebhook, IncomingWebhookRequestDetails, WebhookContext},
};
use hyperswitch_masking::{ExposeInterface, Maskable, PeekInterface};
use transformers as vnpay;


// ─── Connector struct ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Vnpay;

impl Vnpay {
    pub const fn new() -> &'static Self {
        &Self
    }
}

// ─── Marker trait impls ───────────────────────────────────────────────────────

impl api::Payment for Vnpay {}
impl api::PaymentSession for Vnpay {}
impl api::ConnectorAccessToken for Vnpay {}
impl api::MandateSetup for Vnpay {}
impl api::PaymentAuthorize for Vnpay {}
impl api::PaymentSync for Vnpay {}
impl api::PaymentCapture for Vnpay {}
impl api::PaymentVoid for Vnpay {}
impl api::Refund for Vnpay {}
impl api::RefundExecute for Vnpay {}
impl api::RefundSync for Vnpay {}
impl api::PaymentToken for Vnpay {}

// ─── ConnectorCommonExt ───────────────────────────────────────────────────────

impl<Flow, Request, Response> ConnectorCommonExt<Flow, Request, Response> for Vnpay
where
    Self: ConnectorIntegration<Flow, Request, Response>,
{
    fn build_headers(
        &self,
        _req: &RouterData<Flow, Request, Response>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, ConnectorError> {
        Ok(vec![(
            "Content-Type".to_string(),
            "application/json; charset=UTF-8".to_string().into(),
        )])
    }
}

// ─── ConnectorCommon ──────────────────────────────────────────────────────────

impl ConnectorCommon for Vnpay {
    fn id(&self) -> &'static str {
        "vnpay"
    }

    fn get_currency_unit(&self) -> api::CurrencyUnit {
        // VND has no subunit — 1 VND = 1 minor unit
        api::CurrencyUnit::Minor
    }

    fn common_get_content_type(&self) -> &'static str {
        "application/json; charset=UTF-8"
    }

    fn base_url<'a>(&self, connectors: &'a Connectors) -> &'a str {
        connectors.vnpay.base_url.as_ref()
    }

    fn build_error_response(
        &self,
        res: Response,
        _event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, ConnectorError> {
        router_env::logger::info!(connector_response=?res);
        Ok(ErrorResponse {
            status_code: res.status_code,
            code: "VNPAY_ERROR".to_string(),
            message: "VNPay error".to_string(),
            reason: String::from_utf8(res.response.to_vec()).ok(),
            attempt_status: None,
            connector_transaction_id: None,
            connector_response_reference_id: None,
            network_advice_code: None,
            network_decline_code: None,
            network_error_message: None,
            connector_metadata: None,
        })
    }
}

// ─── ConnectorValidation ──────────────────────────────────────────────────────

impl ConnectorValidation for Vnpay {
    fn validate_psync_reference_id(
        &self,
        _data: &PaymentsSyncData,
        _is_three_ds: bool,
        _status: enums::AttemptStatus,
        _connector_meta_data: Option<common_utils::pii::SecretSerdeValue>,
    ) -> CustomResult<(), ConnectorError> {
        Ok(())
    }
}

// ─── Authorize ────────────────────────────────────────────────────────────────

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData> for Vnpay {
    fn get_headers(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_url(
        &self,
        _req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, ConnectorError> {
        Ok(format!(
            "{}paymentv2/vpcpay.html",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        _req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, ConnectorError> {
        // VNPay Authorize is a GET redirect — no request body
        Ok(RequestContent::RawBytes(vec![]))
    }

    fn build_request(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, ConnectorError> {
        // Validate params eagerly — errors surface before any redirect
        let auth = vnpay::VnpayAuthType::try_from(&req.connector_auth_type)?;
        let wrapper = vnpay::VnpayRouterData::from((req.request.amount, req));
        let params = vnpay::VnpayPaymentParams::try_from(&wrapper)?;

        // Build the signed URL and make a lightweight GET so handle_response is invoked.
        // The actual redirect form is assembled inside handle_response using the same data.
        let redirect_url =
            vnpay::build_redirect_url(self.base_url(connectors), &params, auth.hash_secret.peek())?;

        Ok(Some(
            RequestBuilder::new()
                .method(Method::Get)
                .url(&redirect_url)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsAuthorizeRouterData,
        _event_builder: Option<&mut ConnectorEvent>,
        _res: Response,
    ) -> CustomResult<PaymentsAuthorizeRouterData, ConnectorError> {
        let auth = vnpay::VnpayAuthType::try_from(&data.connector_auth_type)?;
        let wrapper = vnpay::VnpayRouterData::from((data.request.amount, data));
        let params = vnpay::VnpayPaymentParams::try_from(&wrapper)?;

        // Get base URL from connector metadata, default to sandbox
        let base_url = data
            .connector_meta_data
            .as_ref()
            .and_then(|meta| {
                serde_json::from_value::<vnpay::VnpayConnectorMeta>(meta.clone().expose()).ok()
            })
            .and_then(|m| m.payment_base_url)
            .unwrap_or_else(|| "https://sandbox.vnpayment.vn/".to_string());

        // Build the signed params map for the redirect form
        let mut map = params.to_sorted_map();
        let sig = vnpay::compute_vnpay_signature(&map, auth.hash_secret.peek())
            .change_context(ConnectorError::RequestEncodingFailed)?;
        map.insert("vnp_SecureHash".to_string(), sig);

        let endpoint = format!("{}paymentv2/vpcpay.html", base_url);

        Ok(PaymentsAuthorizeRouterData {
            status: enums::AttemptStatus::AuthenticationPending,
            response: Ok(PaymentsResponseData::TransactionResponse {
                resource_id: ResponseId::NoResponseId,
                redirection_data: Box::new(Some(RedirectForm::Form {
                    endpoint,
                    method: Method::Get,
                    form_fields: HashMap::from_iter(map),
                })),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: None,
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            }),
            ..data.clone()
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ─── PSync ────────────────────────────────────────────────────────────────────

impl ConnectorIntegration<PSync, PaymentsSyncData, PaymentsResponseData> for Vnpay {
    fn get_headers(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_url(
        &self,
        _req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, ConnectorError> {
        Ok(format!(
            "{}merchant_webapi/api/transaction",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        req: &PaymentsSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, ConnectorError> {
        let query_req = vnpay::VnpayQueryRequest::try_from(req)?;
        Ok(RequestContent::Json(Box::new(query_req)))
    }

    fn build_request(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&PaymentsSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(PaymentsSyncType::get_headers(self, req, connectors)?)
                .set_body(PaymentsSyncType::get_request_body(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsSyncRouterData, ConnectorError> {
        let query_response: vnpay::VnpayQueryResponse = res
            .response
            .parse_struct("VnpayQueryResponse")
            .change_context(ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&query_response));
        router_env::logger::info!(connector_response=?query_response);

        let status = query_response.to_attempt_status();
        let txn_no = query_response.transaction_no.clone().unwrap_or_default();

        Ok(PaymentsSyncRouterData {
            status,
            response: Ok(PaymentsResponseData::TransactionResponse {
                resource_id: ResponseId::ConnectorTransactionId(txn_no),
                redirection_data: Box::new(None),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: query_response.txn_ref,
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            }),
            ..data.clone()
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ─── Unsupported flows (stubs) ────────────────────────────────────────────────

impl ConnectorIntegration<PaymentMethodToken, PaymentMethodTokenizationData, PaymentsResponseData>
    for Vnpay
{
}

impl ConnectorIntegration<Session, PaymentsSessionData, PaymentsResponseData> for Vnpay {}

impl ConnectorIntegration<AccessTokenAuth, AccessTokenRequestData, AccessToken> for Vnpay {}

impl ConnectorIntegration<SetupMandate, SetupMandateRequestData, PaymentsResponseData> for Vnpay {}

impl ConnectorIntegration<Capture, PaymentsCaptureData, PaymentsResponseData> for Vnpay {}

impl ConnectorIntegration<Void, PaymentsCancelData, PaymentsResponseData> for Vnpay {}

impl ConnectorIntegration<Execute, RefundsData, RefundsResponseData> for Vnpay {}

impl ConnectorIntegration<RSync, RefundsData, RefundsResponseData> for Vnpay {}

// ─── Redirect response ────────────────────────────────────────────────────────

impl ConnectorRedirectResponse for Vnpay {
    fn get_flow_type(
        &self,
        _query_params: &str,
        _json_payload: Option<serde_json::Value>,
        action: PaymentAction,
    ) -> CustomResult<CallConnectorAction, ConnectorError> {
        match action {
            PaymentAction::PSync | PaymentAction::CompleteAuthorize => {
                Ok(CallConnectorAction::Trigger)
            }
            PaymentAction::PaymentAuthenticateCompleteAuthorize => Ok(CallConnectorAction::Trigger),
        }
    }
}

// ─── Webhook (IPN) ────────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl IncomingWebhook for Vnpay {
    fn get_webhook_source_verification_algorithm(
        &self,
        _request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn common_utils::crypto::VerifySignature + Send>, ConnectorError> {
        Ok(Box::new(common_utils::crypto::HmacSha512))
    }

    fn get_webhook_source_verification_signature(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _connector_webhook_secrets: &api_models::webhooks::ConnectorWebhookSecrets,
    ) -> CustomResult<Vec<u8>, ConnectorError> {
        let params: BTreeMap<String, String> = serde_urlencoded::from_bytes(request.body)
            .change_context(ConnectorError::WebhookBodyDecodingFailed)?;

        let hash_hex = params
            .get("vnp_SecureHash")
            .ok_or(ConnectorError::WebhookSignatureNotFound)?
            .clone();

        hex::decode(&hash_hex).change_context(ConnectorError::WebhookSignatureNotFound)
    }

    fn get_webhook_source_verification_message(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _merchant_id: &common_utils::id_type::MerchantId,
        _connector_webhook_secrets: &api_models::webhooks::ConnectorWebhookSecrets,
    ) -> CustomResult<Vec<u8>, ConnectorError> {
        let params: BTreeMap<String, String> = serde_urlencoded::from_bytes(request.body)
            .change_context(ConnectorError::WebhookBodyDecodingFailed)?;

        let filtered: BTreeMap<String, String> = params
            .into_iter()
            .filter(|(k, _)| k != "vnp_SecureHash" && k != "vnp_SecureHashType")
            .collect();

        let message = filtered
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        Ok(message.into_bytes())
    }

    fn get_webhook_object_reference_id(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::ObjectReferenceId, ConnectorError> {
        let params: BTreeMap<String, String> = serde_urlencoded::from_bytes(request.body)
            .change_context(ConnectorError::WebhookBodyDecodingFailed)?;

        let txn_ref = params
            .get("vnp_TxnRef")
            .ok_or(ConnectorError::WebhookReferenceIdNotFound)?
            .clone();

        Ok(api_models::webhooks::ObjectReferenceId::PaymentId(
            api_models::payments::PaymentIdType::ConnectorTransactionId(txn_ref),
        ))
    }

    fn get_webhook_event_type(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _context: Option<&WebhookContext>,
    ) -> CustomResult<api_models::webhooks::IncomingWebhookEvent, ConnectorError> {
        let params: BTreeMap<String, String> = serde_urlencoded::from_bytes(request.body)
            .change_context(ConnectorError::WebhookBodyDecodingFailed)?;

        let event = match params.get("vnp_ResponseCode").map(|s| s.as_str()) {
            Some("00") => api_models::webhooks::IncomingWebhookEvent::PaymentIntentSuccess,
            Some("24") => api_models::webhooks::IncomingWebhookEvent::PaymentIntentCancelled,
            _ => api_models::webhooks::IncomingWebhookEvent::PaymentIntentFailure,
        };

        Ok(event)
    }

    fn get_webhook_resource_object(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn hyperswitch_masking::ErasedMaskSerialize>, ConnectorError> {
        let ipn: vnpay::VnpayIpnResponse = serde_urlencoded::from_bytes(request.body)
            .change_context(ConnectorError::WebhookBodyDecodingFailed)?;

        Ok(Box::new(ipn))
    }
}

// ─── ConnectorSpecifications ──────────────────────────────────────────────────

static VNPAY_SUPPORTED_PAYMENT_METHODS: LazyLock<SupportedPaymentMethods> = LazyLock::new(|| {
    let mut methods = SupportedPaymentMethods::new();

    methods.add(
        enums::PaymentMethod::BankRedirect,
        enums::PaymentMethodType::OnlineBankingVietnam,
        PaymentMethodDetails {
            mandates: enums::FeatureStatus::NotSupported,
            refunds: enums::FeatureStatus::NotSupported,
            supported_capture_methods: vec![enums::CaptureMethod::Automatic],
            specific_features: None,
        },
    );

    methods
});

static VNPAY_CONNECTOR_INFO: ConnectorInfo = ConnectorInfo {
    display_name: "VNPay",
    description: "VNPay is Vietnam's leading payment gateway supporting local bank transfers, ATM cards, and QR code payments.",
    connector_type: enums::HyperswitchConnectorCategory::PaymentGateway,
    integration_status: enums::ConnectorIntegrationStatus::Alpha,
};

static VNPAY_SUPPORTED_WEBHOOK_FLOWS: [enums::EventClass; 1] = [enums::EventClass::Payments];

impl ConnectorSpecifications for Vnpay {
    fn get_connector_about(&self) -> Option<&'static ConnectorInfo> {
        Some(&VNPAY_CONNECTOR_INFO)
    }

    fn get_supported_payment_methods(&self) -> Option<&'static SupportedPaymentMethods> {
        Some(&*VNPAY_SUPPORTED_PAYMENT_METHODS)
    }

    fn get_supported_webhook_flows(&self) -> Option<&'static [enums::EventClass]> {
        Some(&VNPAY_SUPPORTED_WEBHOOK_FLOWS)
    }
}
