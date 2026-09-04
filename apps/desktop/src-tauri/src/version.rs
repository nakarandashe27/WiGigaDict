pub const APP_VERSION: &str = "0.0.1-dev";
pub const BUILD_COMMIT: &str = match option_env!("WIGIGADICT_BUILD_COMMIT") {
    Some(value) => value,
    None => "unknown",
};
