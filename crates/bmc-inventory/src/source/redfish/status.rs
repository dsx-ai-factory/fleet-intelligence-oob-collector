use bmc_error::{SanitizedErrorMessage, SanitizedUriReference};

use crate::models::{InventoryCondition, InventoryReference, InventoryStatus};

/// Use the dependency's Serde names, not its snake_case convenience names.
/// Unknown tokens have already been discarded by nv-redfish's enum parser.
pub(super) fn redfish_enum_string(value: impl serde::Serialize) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        serde_json::Value::String(value) if value != "UnsupportedValue" => Some(value),
        _ => None,
    }
}

pub(super) fn status_data(
    status: Option<&nv_redfish::schema::resource::Status>,
) -> Option<InventoryStatus> {
    status.map(|status| InventoryStatus {
        conditions: status
            .conditions
            .as_ref()
            .and_then(Option::as_ref)
            .map(|conditions| conditions.iter().map(map_condition).collect()),
        health: status.health.flatten().and_then(redfish_enum_string),
        health_rollup: status.health_rollup.flatten().and_then(redfish_enum_string),
        state: status.state.flatten().and_then(redfish_enum_string),
    })
}

fn map_condition(condition: &nv_redfish::schema::resource::Condition) -> InventoryCondition {
    InventoryCondition {
        timestamp: condition.timestamp.map(|timestamp| timestamp.to_string()),
        message_id: condition.message_id.clone(),
        message_args: condition
            .message_args
            .clone()
            .map(|args| args.into_iter().map(SanitizedErrorMessage::new).collect()),
        message: condition.message.clone().map(SanitizedErrorMessage::new),
        severity: condition.severity.and_then(redfish_enum_string),
        resolution: condition.resolution.clone().map(SanitizedErrorMessage::new),
        condition_type: condition
            .condition_type
            .flatten()
            .and_then(redfish_enum_string),
        origin_of_condition: condition.origin_of_condition.as_ref().map(|origin| {
            InventoryReference {
                odata_id: SanitizedUriReference::new(origin.odata_id.to_string()),
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_values_keep_redfish_spelling() {
        use nv_redfish::schema::{
            chassis::ChassisType,
            manager::ManagerType,
            pcie_device::DeviceType,
            processor::ProcessorType,
            resource::{ConditionType, Health, PowerState, State},
        };
        assert_eq!(
            redfish_enum_string(ManagerType::Bmc).as_deref(),
            Some("BMC")
        );
        assert_eq!(
            redfish_enum_string(PowerState::PoweringOn).as_deref(),
            Some("PoweringOn")
        );
        assert_eq!(
            redfish_enum_string(ChassisType::RackMount).as_deref(),
            Some("RackMount")
        );
        assert_eq!(
            redfish_enum_string(ProcessorType::Cpu).as_deref(),
            Some("CPU")
        );
        assert_eq!(
            redfish_enum_string(DeviceType::SingleFunction).as_deref(),
            Some("SingleFunction")
        );
        assert_eq!(redfish_enum_string(Health::Ok).as_deref(), Some("OK"));
        assert_eq!(
            redfish_enum_string(State::StandbyOffline).as_deref(),
            Some("StandbyOffline")
        );
        assert_eq!(
            redfish_enum_string(ConditionType::Alert).as_deref(),
            Some("Alert")
        );
        assert_eq!(redfish_enum_string(ManagerType::UnsupportedValue), None);
    }

    #[test]
    fn source_sanitizes_condition() {
        let raw: nv_redfish::schema::resource::Condition =
            serde_json::from_value(serde_json::json!({
                "MessageId": "ResourceEvent.1.0.ResourceErrorsDetected",
                "Message": "request failed at https://user:secret@bmc.example.com/redfish/v1",
                "OriginOfCondition": {
                    "@odata.id": "https://user:secret@bmc.example.com/redfish/v1/Chassis/A?token=secret#part"
                }
            }))
            .expect("condition");
        let condition = map_condition(&raw);
        assert_eq!(
            condition.message.unwrap().as_ref(),
            "request failed at [URL_REDACTED]"
        );
        assert_eq!(
            condition.origin_of_condition.unwrap().odata_id.as_ref(),
            "/redfish/v1/Chassis/A#part"
        );
    }
}
