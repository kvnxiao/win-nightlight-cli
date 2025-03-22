use win_nightlight_lib::{get_nightlight_settings, get_nightlight_state, nightlight_settings::NightlightSettings, nightlight_state::NightlightState};

fn main() {
    let raw_settings = get_nightlight_settings().unwrap();
    let settings_to_bytes = raw_settings.serialize_to_bytes();
    let deserialized_settings = NightlightSettings::deserialize_from_bytes(&settings_to_bytes).unwrap();
    assert_eq!(raw_settings, deserialized_settings);

    let raw_state = get_nightlight_state().unwrap();
    let state_to_bytes = raw_state.serialize_to_bytes();
    let deserialized_state = NightlightState::deserialize_from_bytes(&state_to_bytes).unwrap();
    assert_eq!(raw_state, deserialized_state);
}
