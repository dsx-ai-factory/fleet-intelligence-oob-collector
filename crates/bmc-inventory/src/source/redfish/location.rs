// Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use crate::models::{InventoryLocation, PartLocation, Placement};

use super::status::redfish_enum_string;

pub(super) fn location_data(
    location: Option<&nv_redfish::schema::resource::Location>,
) -> Option<InventoryLocation> {
    location.map(|location| InventoryLocation {
        part_location_context: location.part_location_context.clone().flatten(),
        part_location: part_location_data(location.part_location.as_ref()),
        placement: location.placement.as_ref().map(|placement| Placement {
            rack: placement.rack.clone().flatten(),
            row: placement.row.clone().flatten(),
            room: placement.room.clone().flatten(),
            rack_offset: placement.rack_offset.flatten(),
            rack_offset_units: placement
                .rack_offset_units
                .flatten()
                .and_then(redfish_enum_string),
            additional_info: placement.additional_info.clone().flatten(),
            facility_name: placement.facility_name.clone().flatten(),
        }),
    })
}

fn part_location_data(
    part: Option<&nv_redfish::schema::resource::PartLocation>,
) -> Option<PartLocation> {
    part.map(|part| PartLocation {
        service_label: part.service_label.clone().flatten(),
        location_type: part.location_type.flatten().and_then(redfish_enum_string),
        location_ordinal_value: part.location_ordinal_value.flatten(),
        reference: part.reference.flatten().and_then(redfish_enum_string),
        orientation: part.orientation.flatten().and_then(redfish_enum_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_keeps_nested_context_without_service_label() {
        let raw = serde_json::from_value(serde_json::json!({
            "PartLocation": {"LocationType": "Slot", "LocationOrdinalValue": 0, "Reference": "Front", "Orientation": "LeftToRight"},
            "PartLocationContext": "Drawer 2",
            "Placement": {"RackOffset": 3, "RackOffsetUnits": "OpenU", "FacilityName": "Test facility"}
        })).unwrap();
        let value = location_data(Some(&raw)).unwrap();
        let part = value.part_location.unwrap();
        assert_eq!(part.service_label, None);
        assert_eq!(part.location_type.as_deref(), Some("Slot"));
        assert_eq!(part.location_ordinal_value, Some(0));
        assert_eq!(value.part_location_context.as_deref(), Some("Drawer 2"));
        assert_eq!(
            value.placement.unwrap().rack_offset_units.as_deref(),
            Some("OpenU")
        );
        let raw = serde_json::from_value(serde_json::json!({"PartLocation": {}, "Placement": {}}))
            .unwrap();
        let value = location_data(Some(&raw)).unwrap();
        assert!(value.part_location.is_some());
        assert!(value.placement.is_some());
        assert!(location_data(None).is_none());
    }
}
