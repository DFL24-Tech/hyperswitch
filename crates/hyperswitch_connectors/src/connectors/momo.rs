pub mod transformers;

use std::sync::LazyLock;

use api_models::webhooks::IncomingWebhookEvent;
use common_enums::enums;
use common_utils::{
    crypto,
    errors::CustomResult,
    ext_traits::ByteSliceExt,
    request::{Method, Request, RequestBuilder, RequestContent},
    types::{AmountConvertor, MinorUnit, MinorUnitForConnector},
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    router_data::{AccessToken, ErrorResponse, RouterData},
    router_flow_types::{
        access_token_auth::AccessTokenAuth,
        payments::{Authorize, Capture, PSync, PaymentMethodToken, Session, SetupMandate, Void},
        refunds::{Execute, RSync},
    },
    router_request_types::{
        AccessTokenRequestData, PaymentMethodTokenizationData, PaymentsAuthorizeData,
        PaymentsCancelData, PaymentsCaptureData, PaymentsSessionData, PaymentsSyncData,
        RefundsData, SetupMandateRequestData,
    },
    router_response_types::{
        ConnectorInfo, PaymentMethodDetails, PaymentsResponseData, RefundsResponseData,
        SupportedPaymentMethods, SupportedPaymentMethodsExt,
    },
    types::{
        PaymentsAuthorizeRouterData, PaymentsSyncRouterData,
        RefundSyncRouterData, RefundsRouterData,
    },
};
use hyperswitch_interfaces::{
    api::{
        self, ConnectorCommon, ConnectorCommonExt, ConnectorIntegration, ConnectorSpecifications,
        ConnectorValidation,
    },
    configs::Connectors,
    errors,
    events::connector_api_logs::ConnectorEvent,
    types::{self, Response},
    webhooks::{IncomingWebhook, IncomingWebhookRequestDetails, WebhookContext},
};
use hyperswitch_masking::{Maskable, PeekInterface};
use transformers as momo;

use crate::{constants::headers, types::ResponseRouterData, utils};

#[derive(Clone)]
pub struct Momo {
    amount_converter: &'static (dyn AmountConvertor<Output = MinorUnit> + Sync),
}

impl Momo {
    pub fn new() -> &'static Self {
        &Self {
            amount_converter: &MinorUnitForConnector,
        }
    }
}

impl api::Payment for Momo {}
impl api::PaymentSession for Momo {}
impl api::ConnectorAccessToken for Momo {}
impl api::MandateSetup for Momo {}
impl api::PaymentAuthorize for Momo {}
impl api::PaymentSync for Momo {}
impl api::PaymentCapture for Momo {}
impl api::PaymentVoid for Momo {}
impl api::Refund for Momo {}
impl api::RefundExecute for Momo {}
impl api::RefundSync for Momo {}
impl api::PaymentToken for Momo {}

impl ConnectorIntegration<PaymentMethodToken, PaymentMethodTokenizationData, PaymentsResponseData>
    for Momo
{
}

impl<Flow, Request, Response> ConnectorCommonExt<Flow, Request, Response> for Momo
where
    Self: ConnectorIntegration<Flow, Request, Response>,
{
    fn build_headers(
        &self,
        _req: &RouterData<Flow, Request, Response>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        Ok(vec![(
            headers::CONTENT_TYPE.to_string(),
            self.common_get_content_type().to_string().into(),
        )])
    }
}

impl ConnectorCommon for Momo {
    fn id(&self) -> &'static str {
        "momo"
    }

    fn get_currency_unit(&self) -> api::CurrencyUnit {
        api::CurrencyUnit::Minor
    }

    fn common_get_content_type(&self) -> &'static str {
        "application/json"
    }

    fn base_url<'a>(&self, connectors: &'a Connectors) -> &'a str {
        connectors.momo.base_url.as_ref()
    }

    fn get_auth_header(
        &self,
        _auth_type: &hyperswitch_domain_models::router_data::ConnectorAuthType,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        Ok(vec![])
    }

    fn build_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        let response: momo::MomoErrorResponse = res
            .response
            .parse_struct("MomoErrorResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        Ok(ErrorResponse {
            status_code: res.status_code,
            code: response
                .result_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| hyperswitch_interfaces::consts::NO_ERROR_CODE.to_string()),
            message: response
                .message
                .unwrap_or_else(|| hyperswitch_interfaces::consts::NO_ERROR_MESSAGE.to_string()),
            reason: None,
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

impl ConnectorValidation for Momo {}

impl ConnectorIntegration<Session, PaymentsSessionData, PaymentsResponseData> for Momo {}

impl ConnectorIntegration<AccessTokenAuth, AccessTokenRequestData, AccessToken> for Momo {}

impl ConnectorIntegration<SetupMandate, SetupMandateRequestData, PaymentsResponseData> for Momo {}

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData> for Momo {
    fn get_headers(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!(
            "{}v2/gateway/api/create",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_amount,
            req.request.currency,
        )?;
        let connector_router_data = momo::MomoRouterData::from((amount, req));
        let connector_req = momo::MomoCreatePaymentRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::PaymentsAuthorizeType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::PaymentsAuthorizeType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(types::PaymentsAuthorizeType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsAuthorizeRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsAuthorizeRouterData, errors::ConnectorError> {
        let response: momo::MomoCreatePaymentResponse = res
            .response
            .parse_struct("MomoCreatePaymentResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PSync, PaymentsSyncData, PaymentsResponseData> for Momo {
    fn get_headers(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!(
            "{}v2/gateway/api/query",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        req: &PaymentsSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let connector_req = momo::MomoQueryRequest::try_from(req)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::PaymentsSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::PaymentsSyncType::get_headers(self, req, connectors)?)
                .set_body(types::PaymentsSyncType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsSyncRouterData, errors::ConnectorError> {
        let response: momo::MomoQueryResponse = res
            .response
            .parse_struct("MomoQueryResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<Capture, PaymentsCaptureData, PaymentsResponseData> for Momo {}

impl ConnectorIntegration<Void, PaymentsCancelData, PaymentsResponseData> for Momo {}

impl ConnectorIntegration<Execute, RefundsData, RefundsResponseData> for Momo {
    fn get_headers(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!(
            "{}v2/gateway/api/refund",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        req: &RefundsRouterData<Execute>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let refund_amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_refund_amount,
            req.request.currency,
        )?;
        let connector_router_data = momo::MomoRouterData::from((refund_amount, req));
        let connector_req = momo::MomoRefundRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::RefundExecuteType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::RefundExecuteType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(types::RefundExecuteType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundsRouterData<Execute>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundsRouterData<Execute>, errors::ConnectorError> {
        let response: momo::MomoRefundResponse = res
            .response
            .parse_struct("MomoRefundResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<RSync, RefundsData, RefundsResponseData> for Momo {
    fn get_headers(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!(
            "{}v2/gateway/api/query",
            self.base_url(connectors)
        ))
    }

    fn get_request_body(
        &self,
        req: &RefundSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let connector_req = momo::MomoRefundQueryRequest::try_from(req)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::RefundSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::RefundSyncType::get_headers(self, req, connectors)?)
                .set_body(types::RefundSyncType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundSyncRouterData, errors::ConnectorError> {
        let response: momo::MomoQueryResponse = res
            .response
            .parse_struct("MomoQueryResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

#[async_trait::async_trait]
impl IncomingWebhook for Momo {
    fn get_webhook_source_verification_algorithm(
        &self,
        _request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn crypto::VerifySignature + Send>, errors::ConnectorError>
    {
        Ok(Box::new(crypto::HmacSha256))
    }

    fn get_webhook_source_verification_signature(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _connector_webhook_secrets: &api_models::webhooks::ConnectorWebhookSecrets,
    ) -> CustomResult<Vec<u8>, errors::ConnectorError> {
        let payload: momo::MomoIpnPayload = request
            .body
            .parse_struct("MomoIpnPayload")
            .change_context(errors::ConnectorError::WebhookSignatureNotFound)?;
        hex::decode(payload.signature)
            .change_context(errors::ConnectorError::WebhookSignatureNotFound)
    }

    fn get_webhook_source_verification_message(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _merchant_id: &common_utils::id_type::MerchantId,
        connector_webhook_secrets: &api_models::webhooks::ConnectorWebhookSecrets,
    ) -> CustomResult<Vec<u8>, errors::ConnectorError> {
        let payload: momo::MomoIpnPayload = request
            .body
            .parse_struct("MomoIpnPayload")
            .change_context(errors::ConnectorError::WebhookSourceVerificationFailed)?;

        let access_key = connector_webhook_secrets
            .additional_secret
            .as_ref()
            .map(|s| s.peek().to_string())
            .unwrap_or_default();

        let message = format!(
            "accessKey={}&amount={}&extraData={}&message={}&orderId={}&orderInfo={}&orderType={}&partnerCode={}&payType={}&requestId={}&responseTime={}&resultCode={}&transId={}",
            access_key,
            payload.amount,
            payload.extra_data.as_deref().unwrap_or(""),
            payload.message,
            payload.order_id,
            payload.order_info.as_deref().unwrap_or(""),
            payload.order_type.as_deref().unwrap_or(""),
            payload.partner_code,
            payload.pay_type.as_deref().unwrap_or(""),
            payload.request_id,
            payload.response_time,
            payload.result_code,
            payload.trans_id,
        );
        Ok(message.into_bytes())
    }

    fn get_webhook_object_reference_id(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::ObjectReferenceId, errors::ConnectorError> {
        let payload: momo::MomoIpnPayload = request
            .body
            .parse_struct("MomoIpnPayload")
            .change_context(errors::ConnectorError::WebhookResourceObjectNotFound)?;
        Ok(api_models::webhooks::ObjectReferenceId::PaymentId(
            api_models::payments::PaymentIdType::PaymentAttemptId(payload.order_id),
        ))
    }

    fn get_webhook_event_type(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
        _context: Option<&WebhookContext>,
    ) -> CustomResult<IncomingWebhookEvent, errors::ConnectorError> {
        let payload: momo::MomoIpnPayload = request
            .body
            .parse_struct("MomoIpnPayload")
            .change_context(errors::ConnectorError::WebhookResourceObjectNotFound)?;
        Ok(momo::get_momo_webhook_event(payload.result_code))
    }

    fn get_webhook_resource_object(
        &self,
        request: &IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn hyperswitch_masking::ErasedMaskSerialize>, errors::ConnectorError>
    {
        let payload: momo::MomoIpnPayload = request
            .body
            .parse_struct("MomoIpnPayload")
            .change_context(errors::ConnectorError::WebhookResourceObjectNotFound)?;
        Ok(Box::new(payload))
    }
}

static MOMO_SUPPORTED_PAYMENT_METHODS: LazyLock<SupportedPaymentMethods> =
    LazyLock::new(|| {
        let supported_capture_methods = vec![enums::CaptureMethod::Automatic];

        let mut methods = SupportedPaymentMethods::new();

        methods.add(
            enums::PaymentMethod::Wallet,
            enums::PaymentMethodType::Momo,
            PaymentMethodDetails {
                mandates: enums::FeatureStatus::NotSupported,
                refunds: enums::FeatureStatus::Supported,
                supported_capture_methods,
                specific_features: None,
            },
        );

        methods
    });

static MOMO_CONNECTOR_INFO: ConnectorInfo = ConnectorInfo {
    display_name: "MoMo",
    description: "MoMo is a leading mobile payment service in Vietnam.",
    connector_type: enums::HyperswitchConnectorCategory::PaymentGateway,
    integration_status: enums::ConnectorIntegrationStatus::Beta,
};

static MOMO_SUPPORTED_WEBHOOK_FLOWS: [enums::EventClass; 1] =
    [enums::EventClass::Payments];

impl ConnectorSpecifications for Momo {
    fn get_connector_about(&self) -> Option<&'static ConnectorInfo> {
        Some(&MOMO_CONNECTOR_INFO)
    }

    fn get_supported_payment_methods(&self) -> Option<&'static SupportedPaymentMethods> {
        Some(&*MOMO_SUPPORTED_PAYMENT_METHODS)
    }

    fn get_supported_webhook_flows(&self) -> Option<&'static [enums::EventClass]> {
        Some(&MOMO_SUPPORTED_WEBHOOK_FLOWS)
    }
}
