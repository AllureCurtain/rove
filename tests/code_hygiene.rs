#[test]
fn lib_rs_does_not_hide_dead_code_globally() {
    let lib = std::fs::read_to_string("src/lib.rs").unwrap();

    assert!(!lib.contains("#![allow(dead_code)]"));
}

#[test]
fn runtime_docs_record_phase_12_hygiene_and_source_of_truth_status() {
    let status = std::fs::read_to_string("docs/runtime/implementation-status.md").unwrap();
    let guide = std::fs::read_to_string("docs/runtime/implementation-guide.md").unwrap();
    let readme = std::fs::read_to_string("README.md").unwrap();

    assert!(status.contains("Dead code warnings are enforced"));
    assert!(status.contains("Runtime docs are the source of truth"));
    assert!(guide.contains("Runtime Docs As Source Of Truth"));
    assert!(readme.contains("Current runtime source of truth"));
}

#[test]
fn dev_launcher_documents_process_lifecycle_and_modes() {
    let script = std::fs::read_to_string("scripts/dev.ps1").expect("scripts/dev.ps1 should exist");

    assert!(script.contains("[switch]$Provider"));
    assert!(script.contains("[switch]$InstallWebDeps"));
    assert!(script.contains("[int]$RunSeconds"));
    assert!(script.contains("ROVE_PROVIDER = \"fake\""));
    assert!(script.contains("ROVE_MODEL = \"fake\""));
    assert!(script.contains("Test-PortFree $apiPort \"API\""));
    assert!(script.contains("if ($RunSeconds -gt 0)"));
    assert!(script.contains("Stop-ProcessTree $webProcess"));
    assert!(script.contains("Stop-ProcessTree $apiProcess"));
    assert!(script.contains("http://localhost:$WebPort"));
    assert!(script.contains("Press Ctrl+C to stop API and Web."));
}

#[test]
fn runtime_docs_declare_current_mvp_boundary() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_readme = std::fs::read_to_string(root.join("docs/runtime/README.md")).unwrap();
    let mvp_definition =
        std::fs::read_to_string(root.join("docs/runtime/mvp-definition.md")).unwrap();
    let implementation_status =
        std::fs::read_to_string(root.join("docs/runtime/implementation-status.md")).unwrap();
    let root_readme = std::fs::read_to_string(root.join("README.md")).unwrap();

    assert!(
        runtime_readme.contains("mvp-definition.md"),
        "runtime README should link to the MVP definition"
    );
    assert!(
        root_readme.contains("Current MVP"),
        "root README should expose the current MVP status"
    );
    assert!(
        implementation_status.contains("MVP Status"),
        "implementation status should expose the MVP status"
    );
    assert!(
        mvp_definition.contains("MVP reached"),
        "MVP definition should explicitly declare the reached state"
    );
    assert!(
        mvp_definition.contains("Out of scope"),
        "MVP definition should name exclusions"
    );
    assert!(
        mvp_definition.contains("Browser/Desktop"),
        "MVP definition should keep future workspace surfaces out of scope"
    );
}

#[test]
fn runtime_docs_index_release_readiness_checklist() {
    let runtime_readme = std::fs::read_to_string("docs/runtime/README.md").unwrap();
    let checklist = std::fs::read_to_string("docs/runtime/release-readiness.md")
        .expect("release readiness checklist should exist");

    assert!(runtime_readme.contains("release-readiness.md"));
    assert!(checklist.contains("Deterministic Gates"));
    assert!(checklist.contains("Local-Full Integration"));
    assert!(checklist.contains("Provider Smoke"));
    assert!(checklist.contains("Security Posture"));
    assert!(checklist.contains("Out-of-scope"));
}

#[test]
fn provider_integration_runner_is_generic_and_documented() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");
    let env_example = std::fs::read_to_string(".env.integration.example").unwrap();
    let provider_docs = std::fs::read_to_string("docs/runtime/provider-smoke.md").unwrap();
    let integration_docs = std::fs::read_to_string("docs/runtime/integration-testing.md").unwrap();

    assert!(script.contains("[string]$Provider"));
    assert!(script.contains("[string]$Model"));
    assert!(script.contains("[string]$ApiBase"));
    assert!(script.contains("[string]$ApiKeyEnv"));
    assert!(script.contains("[string]$ModelsEndpoint"));
    assert!(script.contains("[switch]$SkipModelInventory"));
    assert!(script.contains("[switch]$SkipProviderSmoke"));
    assert!(script.contains("[switch]$SkipApiSmoke"));
    assert!(script.contains("[switch]$SkipWebSmoke"));
    assert!(script.contains("[switch]$RunStress"));
    assert!(script.contains("[switch]$RunRestartRecovery"));
    assert!(script.contains("[switch]$RunLongSoak"));
    assert!(script.contains("[switch]$RunExternalMcp"));
    assert!(script.contains("[int]$StressSequentialCount"));
    assert!(script.contains("[int]$StressConcurrentCount"));
    assert!(script.contains("[int]$StressJobTimeoutSeconds"));
    assert!(script.contains("[int]$RestartRecoveryTimeoutSeconds"));
    assert!(script.contains("[int]$LongSoakCount"));
    assert!(script.contains("[int]$LongSoakDelayMs"));
    assert!(script.contains("[string]$ExternalMcpToolName"));
    assert!(script.contains("openai-compatible"));
    assert!(script.contains("Invoke-ProviderSmoke"));
    assert!(script.contains("Invoke-ApiSmoke"));
    assert!(script.contains("Invoke-WebSmoke"));
    assert!(script.contains("Invoke-StressGate"));
    assert!(script.contains("Invoke-RestartRecoveryGate"));
    assert!(script.contains("Invoke-LongSoakGate"));
    assert!(script.contains("Classify-RunReport"));
    assert!(script.contains("Invoke-ExternalMcpGate"));
    assert!(script.contains("ROVE_MCP_CONFIG"));
    assert!(script.contains("mcp_servers.example.json"));
    assert!(script.contains("external-mcp-config.redacted.json"));
    assert!(script.contains("evidence-summary.json"));
    assert!(script.contains("stress-runs-before-restart.json"));
    assert!(script.contains("stress-runs-after-restart.json"));
    assert!(script.contains("stress-sequential-$i.state.json"));
    assert!(script.contains("stress-concurrent-$($i + 1).state.json"));
    assert!(script.contains("long-soak-summary.json"));
    assert!(script.contains("long-soak-$i.state.json"));
    assert!(script.contains("key_present"));
    assert!(!script.contains("SiliconFlow only"));

    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_PROVIDER=openai-compatible"));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_MODEL="));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_API_BASE="));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_API_KEY_ENV=OPENAI_API_KEY"));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS=180"));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS=90"));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT=20"));
    assert!(env_example.contains("ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS=500"));

    assert!(provider_docs.contains("scripts/provider-integration.ps1"));
    assert!(provider_docs.contains("official OpenAI-compatible APIs"));
    assert!(provider_docs.contains("relay or gateway APIs"));
    assert!(provider_docs.contains("-RunStress"));
    assert!(provider_docs.contains("-RunExternalMcp"));
    assert!(integration_docs.contains("provider-integration.ps1"));
}

#[test]
fn provider_integration_runner_supports_native_provider_protocols() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");
    let provider_docs = std::fs::read_to_string("docs/runtime/provider-smoke.md").unwrap();
    let readiness = std::fs::read_to_string("docs/runtime/release-readiness.md").unwrap();

    assert!(script.contains("function Normalize-ProviderName"));
    assert!(script.contains("function Invoke-AnthropicModelInventory"));
    assert!(script.contains("function Invoke-OllamaModelInventory"));
    assert!(script.contains("function Invoke-ProviderRestMethod"));
    assert!(script.contains("Provider request to $Uri failed:"));
    assert!(script.contains("-Uri $endpoint"));
    assert!(script.contains("Provider request to .* failed|Connect"));
    assert!(script.contains("anthropic_real_provider_smoke_when_enabled"));
    assert!(script.contains("ollama_real_provider_smoke_when_enabled"));
    assert!(script.contains("provider = @{"));
    assert!(script.contains("name = $Provider"));
    assert!(script.contains("api_key_env = $ApiKeyEnv"));
    assert!(script.contains("if ($normalized -eq \"ollama\")"));
    assert!(script.contains("return \"\""));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_ANTHROPIC"));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_OLLAMA"));
    assert!(!script.contains("currently automates API/Web gates for openai-compatible providers"));

    assert!(provider_docs.contains("-Provider anthropic"));
    assert!(provider_docs.contains("-Provider ollama"));
    assert!(readiness.contains("Provider Gate Matrix"));
}

#[test]
fn provider_integration_runner_records_requested_gate_failures() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(script.contains("function New-RequestedGateStatusMap"));
    assert!(script.contains("function Invoke-Gate"));
    assert!(script.contains("$script:CurrentGateName"));
    assert!(script.contains("failure-classification.json"));
    assert!(script.contains("failed_gate = $script:CurrentGateName"));
    assert!(script.contains("classification = $classification"));
    assert!(script.contains("message = $Message"));
    assert!(script.contains("$Gates[$GateName] = \"failed\""));
    assert!(script.contains("$gates = New-RequestedGateStatusMap"));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"model_inventory\""));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"provider_smoke\""));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"provider_full_api\""));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"web_provider\""));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"stress\""));
    assert!(script.contains("Invoke-Gate -Gates $gates -GateName \"external_mcp\""));
    assert!(!script.contains("$gates[\"failure\"] = $_.Exception.Message"));
}

#[test]
fn provider_integration_runner_keeps_ollama_keyless_after_env_import() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(script.contains("if ($Provider -eq \"ollama\")"));
    assert!(script.contains("$ApiKeyEnv = \"\""));
    assert!(script.contains("providerKeyEnv: providerKeyEnv || ''"));
}

#[test]
fn provider_integration_runner_reuses_gate_classification_artifacts() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(script.contains("function Read-GateClassification"));
    assert!(script.contains("provider-smoke-result.json"));
    assert!(script.contains("return [string]$result.classification"));
    assert!(script.contains("Read-GateClassification -GateName $script:CurrentGateName"));
    assert!(script.contains("if (-not $classification)"));
}
