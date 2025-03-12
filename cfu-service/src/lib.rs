#![no_std]
use embassy_sync::once_lock::OnceLock;

use embedded_cfu_protocol::client::CfuReceiveContent;
use embedded_cfu_protocol::protocol_definitions::*;

use embedded_services::cfu::component::*;
use embedded_services::cfu::{CfuError, ContextToken};
use embedded_services::{comms, error, info};
use heapless::Vec;

pub mod host;

pub struct CfuClient {
    /// Cfu Client context
    context: ContextToken,
    /// Comms endpoint
    tp: comms::Endpoint,
}

/// use default "do-nothing" implementations
impl<T, C, E: Default> CfuReceiveContent<T, C, E> for CfuClient {}

impl CfuClient {
    /// Create a new Cfu Client
    pub fn create() -> Option<Self> {
        Some(Self {
            context: ContextToken::create()?,
            tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Nonvol)),
        })
    }
    pub async fn process_request(&self) -> Result<(), CfuError> {
        let request = self.context.wait_request().await;
        //let device = self.context.get_device(request.id).await?;
        let comp = request.id;

        match request.data {
            RequestData::FwVersionRequest => {
                info!("Received FwVersionRequest, comp {}", comp);
                if let Ok(device) = self.context.get_device(comp).await {
                    let resp = device
                        .execute_device_request(request.data)
                        .await
                        .map_err(CfuError::ProtocolError)?;

                    // TODO replace with signal to component to get its own fw version
                    //cfu::send_request(comp, RequestData::FwVersionRequest).await?;
                    match resp {
                        InternalResponseData::FwVersionResponse(r) => {
                            let ver = r.component_info[0].fw_version;
                            info!("got fw version {:?} for comp {}", ver, comp);
                        }
                        _ => {
                            error!("Invalid response to get fw version {:?} from comp {}", resp, comp);
                            return Err(CfuError::ProtocolError(CfuProtocolError::BadResponse));
                        }
                    }
                    self.context.send_response(resp).await;
                    return Ok(());
                }
                Err(CfuError::InvalidComponent)
            }
            RequestData::PrimaryNeedsSubcomponentFwVersion(ids) => {
                info!("Received PrimaryNeedsSubcomponentFwVersion, comp {}", comp);
                // verify that the primary component is valid
                if let Ok(_device) = self.context.get_device(comp).await {
                    let mut fwver_vec: Vec<FwVerComponentInfo, MAX_SUBCMPT_COUNT> = Vec::new();
                    for id in ids {
                        let sub_device = self.context.get_device(id).await?;
                        let resp = sub_device
                            .execute_device_request(RequestData::FwVersionRequest)
                            .await
                            .map_err(CfuError::ProtocolError)?;
                        match resp {
                            InternalResponseData::FwVersionResponse(r) => {
                                let ver = r.component_info[0].fw_version;
                                info!("got fw version {:?} for comp {}", ver, id);
                                let _ = fwver_vec.push(r.component_info[0]);
                            }
                            _ => {
                                error!("Invalid response to get fw version from comp {}", id);
                                return Err(CfuError::ProtocolError(CfuProtocolError::BadResponse));
                            }
                        }
                    }
                    // use unwrap_or_default in case a primary component doesn't the max num of subcomponents
                    self.context
                        .send_response(InternalResponseData::SubcomponentFwVersionResponse(
                            fwver_vec.into_array().unwrap_or_default(),
                        ))
                        .await;
                    return Ok(());
                }
                Err(CfuError::InvalidComponent)
            }
            RequestData::GiveContent(_content_cmd) => Ok(()),
            RequestData::GiveOffer(_offer_cmd) => Ok(()),
            RequestData::PrepareComponentForUpdate => Ok(()),
            RequestData::FinalizeUpdate => Ok(()),
        }
    }
}

impl comms::MailboxDelegate for CfuClient {
    fn receive(&self, _message: &comms::Message) {}
}

#[embassy_executor::task]
pub async fn task() {
    info!("Starting cfu client task");
    static CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfuclient = CLIENT.get_or_init(|| CfuClient::create().expect("cfu client singleton already initialized"));

    if comms::register_endpoint(cfuclient, &cfuclient.tp).await.is_err() {
        error!("Failed to register cfu endpoint");
        return;
    }

    loop {
        if let Err(e) = cfuclient.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
