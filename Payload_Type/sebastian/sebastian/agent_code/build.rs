fn main() {
    // Compile-time configuration injection (replaces Go ldflags)
    // These environment variables are set by the Mythic container during payload build
    println!("cargo:rerun-if-env-changed=AGENT_UUID");
    println!("cargo:rerun-if-env-changed=EGRESS_ORDER");
    println!("cargo:rerun-if-env-changed=EGRESS_FAILOVER");
    println!("cargo:rerun-if-env-changed=FAILED_CONNECTION_COUNT_THRESHOLD");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=PROXY_BYPASS");
    println!("cargo:rerun-if-env-changed=SEBASTIAN_CRATE_TYPE");
    println!("cargo:rerun-if-env-changed=C2_HTTP_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_WEBSOCKET_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_TCP_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_DNS_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_DYNAMICHTTP_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_HTTPX_INITIAL_CONFIG");
    println!("cargo:rerun-if-env-changed=C2_WEBSHELL_INITIAL_CONFIG");

    // Build protobuf definitions for DNS profile
    let proto_path = "proto/dns.proto";
    if std::path::Path::new(proto_path).exists() {
        prost_build::compile_protos(&[proto_path], &["proto/"])
            .expect("Failed to compile protobuf definitions");
    }
}
