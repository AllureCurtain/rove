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
    let env_example = std::fs::read_to_string(".env.integration.example").unwrap();
    let provider_docs = std::fs::read_to_string("docs/runtime/provider-smoke.md").unwrap();
    let readiness = std::fs::read_to_string("docs/runtime/release-readiness.md").unwrap();

    assert!(script.contains("function Normalize-ProviderName"));
    assert!(script.contains("function Invoke-AnthropicModelInventory"));
    assert!(script.contains("function Invoke-OllamaModelInventory"));
    assert!(script.contains("function Invoke-ProviderRestMethod"));
    assert!(script.contains("Provider request to $Uri failed:"));
    assert!(script.contains("-Uri $endpoint"));
    assert!(
        script
            .contains("Provider request to .* failed|request failed|error sending request|Connect")
    );
    assert!(script.contains("anthropic_real_provider_smoke_when_enabled"));
    assert!(script.contains("ollama_real_provider_smoke_when_enabled"));
    assert!(script.contains("provider = @{"));
    assert!(script.contains("name = $Provider"));
    assert!(script.contains("api_key_env = $ApiKeyEnv"));
    assert!(script.contains("if ($normalized -eq \"ollama\")"));
    assert!(script.contains("return \"\""));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_ANTHROPIC"));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_OLLAMA"));
    assert!(script.contains("openai-responses"));
    assert!(script.contains("openai_responses_real_provider_smoke_when_enabled"));
    assert!(!script.contains("currently automates API/Web gates for openai-compatible providers"));

    assert!(env_example.contains("ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES"));
    assert!(provider_docs.contains("-Provider anthropic"));
    assert!(provider_docs.contains("-Provider ollama"));
    assert!(provider_docs.contains("-Provider openai-responses"));
    assert!(readiness.contains("Provider Gate Matrix"));
    assert!(readiness.contains("OpenAI Responses official API"));
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
fn integration_runners_allow_web_origins_before_api_start() {
    let local_script = std::fs::read_to_string("scripts/integration-smoke.ps1")
        .expect("scripts/integration-smoke.ps1 should exist");
    let provider_script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(local_script.contains("function Add-CorsOrigins"));
    assert!(local_script.contains("ROVE_API_CORS_ORIGINS"));
    let local_cors_index = local_script
        .find("Add-CorsOrigins @($WebBase, \"http://localhost:$WebPort\")")
        .expect("local-full runner should allow both 127.0.0.1 and localhost web origins");
    let local_api_start_index = local_script
        .find("Start-BackgroundCommand -Command $apiBinary")
        .expect("local-full runner should start rove-api");
    assert!(
        local_cors_index < local_api_start_index,
        "local-full runner must set CORS origins before starting rove-api"
    );

    assert!(provider_script.contains("function Add-CorsOrigins"));
    assert!(provider_script.contains("ROVE_API_CORS_ORIGINS"));
    let provider_web_index = provider_script
        .find("function Invoke-WebSmoke")
        .expect("provider runner should define a Web smoke gate");
    let provider_cors_index = provider_script[provider_web_index..]
        .find("Add-CorsOrigins @($WebBase, \"http://localhost:$WebPort\")")
        .map(|offset| provider_web_index + offset)
        .expect("provider Web runner should allow both 127.0.0.1 and localhost web origins");
    let provider_api_start_index = provider_script[provider_web_index..]
        .find("web-provider-api.out.log")
        .map(|offset| provider_web_index + offset)
        .expect("provider Web runner should start rove-api for Web smoke");
    assert!(
        provider_cors_index < provider_api_start_index,
        "provider Web runner must set CORS origins before starting rove-api"
    );
}

#[test]
fn local_full_runner_builds_rove_api_before_starting_service() {
    let script = std::fs::read_to_string("scripts/integration-smoke.ps1")
        .expect("scripts/integration-smoke.ps1 should exist");

    let build_args_index = script
        .find("$apiBuildArgs = @(\"build\", \"--bin\", \"rove-api\")")
        .expect("local-full runner should build rove-api explicitly");
    let build_location_index = script
        .find("Push-Location $RepoRoot")
        .expect("local-full runner should build from the repository root");
    let build_invoke_index = script
        .find("& cargo @apiBuildArgs")
        .expect("local-full runner should run the rove-api build before startup");
    let pop_location_index = script[build_location_index..]
        .find("Pop-Location")
        .map(|offset| build_location_index + offset)
        .expect("local-full runner should restore the previous location after building");
    let binary_path_index = script
        .find("$apiBinary = Join-Path")
        .expect("local-full runner should resolve the compiled rove-api binary");
    let start_index = script
        .find("Start-BackgroundCommand -Command $apiBinary")
        .expect("local-full runner should start the compiled rove-api binary");

    assert!(
        build_args_index < build_invoke_index,
        "local-full runner should define build args before invoking cargo"
    );
    assert!(
        build_location_index < build_invoke_index,
        "local-full runner should switch to the repository root before building"
    );
    assert!(
        build_invoke_index < pop_location_index,
        "local-full runner should restore the previous location after the build command"
    );
    assert!(
        pop_location_index < binary_path_index,
        "local-full runner should build before resolving the compiled binary"
    );
    assert!(
        binary_path_index < start_index,
        "local-full runner should resolve the compiled binary before starting it"
    );
    assert!(
        !script.contains("$apiArgs = @(\"run\", \"--bin\", \"rove-api\""),
        "local-full runner should not include Cargo execution in the API readiness window"
    );
    assert!(
        !script.contains("Start-BackgroundCommand -Command \"cargo\" -Arguments $apiArgs"),
        "local-full runner should not start rove-api through cargo run"
    );
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

#[test]
fn provider_integration_runner_classifies_transport_failures_before_tool_fields() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(script.contains("error sending request"));
    assert!(script.contains("request failed"));
    assert!(script.contains("did not emit an echo tool call"));
    assert!(!script.contains("tool_call|tool call"));

    let network_index = script.find("error sending request").unwrap();
    let tool_index = script.find("did not emit an echo tool call").unwrap();
    assert!(
        network_index < tool_index,
        "network/transport failures must be classified before tool-use wording"
    );
}

#[test]
fn provider_integration_runner_does_not_match_status_codes_inside_paths() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(
        !script.contains("\"401|403|Unauthorized"),
        "provider classification must not match bare status-code substrings inside paths or hashes"
    );
    assert!(script.contains("\\b(401|403)\\b"));
    assert!(script.contains("did not emit an echo tool call"));
}

#[test]
fn provider_integration_runner_classifies_tool_assertions_before_panic_text() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    let tool_index = script.find("did not emit an echo tool call").unwrap();
    let panic_index = script.find("panic|SQLite").unwrap();
    assert!(
        tool_index < panic_index,
        "provider smoke assertion failures include panic text, so tool-use behavior must win first"
    );
}

#[test]
fn provider_integration_runner_writes_stress_summary_on_long_soak_failure() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");

    assert!(script.contains("function Write-StressSummary"));
    assert!(script.contains("-FailedGate \"long_soak\""));
    assert!(script.contains("-LongSoakStatus \"failed\""));
    assert!(script.contains("long_soak_summary = \"long-soak-summary.json\""));
    assert!(script.contains("Write-StressSummary -CreatedJobs $created -RestartStatus $restartStatus -LongSoakStatus \"failed\""));
}

#[test]
fn runtime_docs_explain_plan_react_core() {
    let doc = std::fs::read_to_string("docs/runtime/react-loop.md").unwrap();
    let runtime_readme = std::fs::read_to_string("docs/runtime/README.md").unwrap();
    let root_readme = std::fs::read_to_string("README.md").unwrap();

    assert!(doc.contains("Plan Outside, ReAct Inside"));
    assert!(doc.contains("run_unplanned_loop"));
    assert!(doc.contains("run_planned_loop"));
    assert!(doc.contains("run_model_turn"));
    assert!(doc.contains("run_tool_turn"));
    assert!(doc.contains("ReactTurn"));
    assert!(runtime_readme.contains("react-loop.md"));
    assert!(root_readme.contains("react-loop.md"));
}

#[test]
fn benchmark_evidence_format_is_documented() {
    let results = std::fs::read_to_string("benchmarks/results/README.md").unwrap();
    let docs = std::fs::read_to_string("docs/runtime/benchmark-evidence.md").unwrap();

    assert!(results.contains("DATA_PROVENANCE.md"));
    assert!(results.contains("rove-benchmark-core-report.md"));
    assert!(results.contains("metrics.json"));
    assert!(docs.contains("harness regression"));
    assert!(docs.contains("recovery/resume ablation"));
}
