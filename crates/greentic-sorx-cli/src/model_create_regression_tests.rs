// Synthetic contract reproducing parent/child and business-action overlap.
fn bulk_record_runtime() -> HttpRuntime {
    // Record names/types/required flags extracted from the supplied contract;
    // no uploaded business data is stored in this fixture.
    let mut model: Value = serde_json::from_str(include_str!(
        "../tests/e2e/fixtures/manager/bulk-record-schema/model.json"
    ))
    .unwrap();
    let record = model["records"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["name"] == "source_system")
        .unwrap();
    record["access"] = json!({"create":{"roles":{"all_of":["writer","operator"]}}});
    let endpoints = [
        ("apply_source_change_batch", "source_system", "canonical_record_id"),
        ("import_source_snapshot", "source_system", "canonical_record_id"),
        ("create_source_observation", "source_observation", "canonical_record_id"),
        ("create_network_interface", "network_element", "interface_name"),
        ("create_source_system", "source_system", "source_name"),
    ].map(|(name, entity, field)| json!({
        "endpoint_id":name,"operation_id":name,"operation":if name.starts_with("create_") { "create" } else { "command" }, "method":"POST",
        "path":format!("/v1/test/{name}"),"entity":entity,"collection": if name == "apply_source_change_batch" { "business_action_audit".to_string() } else { format!("{entity}s") },"provider_binding":"store",
        "input_schema":{"type":"object","required":[field],"properties":{(field):{"type":"string"}}}
    }));
    runtime_with_gateway_and_model(
        json!({"schema":"greentic.sorla.agent-gateway.v1",
        "record_hierarchy":{"source_system":{"main":true},"source_observation":{"parent":"source_system","field":"source_system_id"},
        "network_element":{"main":true},"network_interface":{"parent":"network_element","field":"network_element_id"}},
        "endpoints":endpoints}),
        model,
    )
}

fn bulk_headers() -> Vec<(&'static str, &'static str)> {
    let mut headers = tenant_headers().to_vec();
    headers.push(("X-Greentic-Caller-Role", "writer,operator"));
    headers
}

fn bulk_submit(runtime: &HttpRuntime, record: &str, input: Value) -> HttpResponse {
    response(
        runtime,
        "POST",
        "/v1/sorx/manager/submit",
        &bulk_headers(),
        &json!({"record":record,"endpoint_id":format!("{record}.model_create"),
        "operation_id":format!("{record}.model_create"),"input":input})
        .to_string(),
    )
}

#[test]
fn bulk_model_schema_excludes_business_inputs_and_keeps_optional_fields_optional() {
    let runtime = bulk_record_runtime();
    // Reproduce the original contamination without mutating the source model.
    let req = HttpRequest {
        method: "GET".into(),
        path: "/v1/sorx/manager/view".into(),
        headers: bulk_headers()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: String::new(),
    };
    let context = runtime.manager_context(&req).unwrap();
    let raw = generate_manager_view(&context, &runtime.runtime.router, &runtime.runtime.policy);
    assert!(
        raw.records
            .iter()
            .find(|r| r.record == "source_system")
            .unwrap()
            .fields
            .iter()
            .any(|f| f.name == "canonical_record_id" && f.required)
    );
    let view = request(
        &runtime,
        "GET",
        "/v1/sorx/manager/view",
        &bulk_headers(),
        "",
    );
    for (record, alien) in [
        ("source_system", "canonical_record_id"),
        ("network_element", "interface_name"),
    ] {
        let fields = view["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["record"] == record)
            .unwrap();
        assert!(
            !fields["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == alien)
        );
        assert!(
            view["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["record"] == record
                    && a["endpoint_id"] == format!("{record}.model_create"))
        );
    }
    let child = view["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["record"] == "source_observation")
        .unwrap();
    assert_eq!(
        child["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "canonical_record_id")
            .unwrap()["required"],
        false
    );
}

#[test]
fn bulk_model_creates_parents_and_children_with_own_fields_and_preserves_uuids() {
    let runtime = bulk_record_runtime();
    let uuid = "00000000-0000-4000-8000-000000000001";
    for record in [
        "source_system",
        "source_observation",
        "network_element",
        "network_interface",
    ] {
        let mut input = json!({});
        for field in &runtime.manager_model_records[record].fields {
            if !field.required {
                continue;
            }
            let value = if field.name.ends_with("_id") {
                json!(uuid)
            } else {
                match field.json_type.as_deref() {
                    Some("integer") => json!(15),
                    Some("datetime" | "timestamp" | "date") => json!("2026-01-01T00:00:00Z"),
                    Some("boolean") => json!(true),
                    _ => json!("Synthetic value"),
                }
            };
            input[&field.name] = value;
        }
        let result = bulk_submit(&runtime, record, input.clone());
        assert_eq!(result.status, 200, "{record}: {}", result.body);
        let stored = runtime.manager_record_rows("tenant-a", record);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].entity, record);
        assert_eq!(
            stored[0].collection,
            runtime.manager_model_records[record].collection
        );
        for (key, value) in input.as_object().unwrap() {
            assert_eq!(&result.body["result"]["data"][key], value);
        }
    }
}

#[test]
fn bulk_model_rejects_own_schema_errors_and_enforces_record_create_roles() {
    let runtime = bulk_record_runtime();
    let invalid = bulk_submit(&runtime, "source_system", json!({"source_system_id":"id"}));
    assert_eq!(invalid.status, 400);
    assert!(invalid.body.to_string().contains("source_name"));
    assert!(!invalid.body.to_string().contains("canonical_record_id"));
    let foreign = bulk_submit(
        &runtime,
        "source_system",
        json!({"source_name":"source","canonical_record_id":"alien"}),
    );
    assert_eq!(foreign.status, 400);
    let denied = response(&runtime,"POST","/v1/sorx/manager/submit",&tenant_headers(),
        &json!({"record":"source_system","endpoint_id":"source_system.model_create","input":{"source_name":"no permission"}}).to_string());
    assert_eq!(denied.status, 403);
    let mut partial_roles = tenant_headers().to_vec();
    partial_roles.push(("X-Greentic-Caller-Role", "writer"));
    let partial = response(&runtime,"POST","/v1/sorx/manager/submit",&partial_roles,
        &json!({"record":"source_system","endpoint_id":"source_system.model_create","input":{"source_name":"not all roles"}}).to_string());
    assert_eq!(partial.status, 403);
    assert!(
        runtime
            .manager_record_rows("tenant-a", "source_system")
            .is_empty()
    );
    let mismatch = bulk_submit(&runtime, "source_system", json!({"source_name":7}));
    assert_eq!(mismatch.status, 400);
}

#[test]
fn bulk_model_accepts_diagnostic_source_system_shape_without_canonical_id() {
    let runtime = bulk_record_runtime();
    let input = json!({"source_system_id":"00000000-0000-4000-8000-000000000001",
        "source_name":"Synthetic source","managed_domains":"[\"mobile\"]",
        "expected_refresh_interval_minutes":15,"last_successful_sync_at":"2026-01-01T00:00:00Z",
        "data_freshness_status":"FRESH"});
    let result = bulk_submit(&runtime, "source_system", input.clone());
    assert_eq!(result.status, 200, "{}", result.body);
    for (field, value) in input.as_object().unwrap() {
        assert_eq!(&result.body["result"]["data"][field], value);
    }
    assert!(
        result.body["result"]["data"]
            .get("canonical_record_id")
            .is_none()
    );
}

#[test]
fn bulk_model_rejects_record_and_operation_mismatches_before_writing() {
    let runtime = bulk_record_runtime();
    for (record, operation) in [
        ("network_element", "source_system.model_create"),
        ("source_system", "import_source_snapshot"),
    ] {
        let result = response(&runtime,"POST","/v1/sorx/manager/submit",&bulk_headers(),
            &json!({"record":record,"endpoint_id":"source_system.model_create","operation_id":operation,
                "input":{"source_system_id":"00000000-0000-4000-8000-000000000001","source_name":"Synthetic source",
                    "managed_domains":"[]","expected_refresh_interval_minutes":15,"data_freshness_status":"FRESH"}}).to_string());
        assert_eq!(result.status, 400);
        assert!(result.body.to_string().contains("exact model action"));
    }
    assert!(
        runtime
            .manager_record_rows("tenant-a", "source_system")
            .is_empty()
    );
}
