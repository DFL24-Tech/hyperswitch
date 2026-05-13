use router::types::{self, storage::enums};
use test_utils::connector_auth;

use crate::utils::{self, ConnectorActions};

#[derive(Clone, Copy)]
struct VnpayTest;
impl ConnectorActions for VnpayTest {}
impl utils::Connector for VnpayTest {
    fn get_data(&self) -> types::api::ConnectorData {
        use router::connector::Vnpay;
        utils::construct_connector_data_old(
            Box::new(Vnpay::new()),
            types::Connector::Vnpay,
            types::api::GetToken::Connector,
            None,
        )
    }

    fn get_auth_token(&self) -> types::ConnectorAuthType {
        utils::to_connector_auth_type(
            connector_auth::ConnectorAuthentication::new()
                .vnpay
                .expect("Missing connector authentication configuration")
                .into(),
        )
    }

    fn get_name(&self) -> String {
        "vnpay".to_string()
    }
}

static CONNECTOR: VnpayTest = VnpayTest {};

fn get_default_payment_info() -> Option<utils::PaymentInfo> {
    None
}

fn payment_method_details() -> Option<types::PaymentsAuthorizeData> {
    None
}

#[actix_web::test]
async fn should_only_authorize_payment() {
    let response = CONNECTOR
        .authorize_payment(payment_method_details(), get_default_payment_info())
        .await
        .expect("Authorize payment response");
    assert_eq!(response.status, enums::AttemptStatus::AuthenticationPending);
}
