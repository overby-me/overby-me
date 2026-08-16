//! Batch operations for the tramp-agent RPC server.
//!
//! Implements the following RPC method:
//!
//! | Method  | Description                                              |
//! |---------|----------------------------------------------------------|
//! | `batch` | Execute multiple RPC operations in a single round-trip   |
//!
//! The batch method accepts an array of sub-requests, dispatches each one
//! through the normal handler registry, and returns an array of results in
//! the same order.  This is the primary mechanism for amortising network
//! latency — the client can bundle N operations into a single message and
//! get N results back in one response.
//!
//! Sub-requests within a batch are executed **sequentially** by default.
//! An optional `parallel: true` flag causes them to be executed concurrently
//! using `tokio::spawn`.

use rmpv::Value;
use std::sync::Arc;

use crate::ops;
use crate::ops::process::ProcessTable;
use crate::ops::watch::WatchState;
use crate::rpc::{Response, error_code};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required array parameter from a MsgPack map by key.
fn get_array_param<'a>(params: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_array())
    })
}

/// Extract an optional boolean parameter from a MsgPack map by key.
fn get_bool_param(params: &Value, key: &str) -> Option<bool> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_bool())
    })
}

/// Extract a string field from a MsgPack map value.
fn get_str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_str())
    })
}

/// Extract a u64 field from a MsgPack map value.
fn get_u64_field(value: &Value, key: &str) -> Option<u64> {
    value.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_u64())
    })
}

/// Extract the `params` field from a sub-request map, defaulting to an
/// empty map if absent.
fn get_params_field(value: &Value) -> Value {
    value
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some("params"))
                .map(|(_, v)| v.clone())
        })
        .unwrap_or_else(|| Value::Map(vec![]))
}

// ---------------------------------------------------------------------------
// Single sub-request dispatch
// ---------------------------------------------------------------------------

/// Dispatch a single sub-request to the appropriate handler.
///
/// This mirrors the top-level dispatch in `main.rs` but operates on a
/// sub-request value rather than a full [`Request`] message.
///
/// The `sub_id` is used for error reporting only (the batch response uses
/// array ordering, not individual IDs).
async fn dispatch_one(
    method: &str,
    params: &Value,
    sub_id: u64,
    process_table: &ProcessTable,
    watch_state: &WatchState,
) -> Response {
    match method {
        // File operations
        "file.stat" => ops::file::stat(sub_id, params).await,
        "file.stat_batch" => ops::file::stat_batch(sub_id, params).await,
        "file.truename" => ops::file::truename(sub_id, params).await,
        "file.read" => ops::file::read(sub_id, params).await,
        "file.write" => ops::file::write(sub_id, params).await,
        "file.copy" => ops::file::copy(sub_id, params).await,
        "file.rename" => ops::file::rename(sub_id, params).await,
        "file.delete" => ops::file::delete(sub_id, params).await,
        "file.set_modes" => ops::file::set_modes(sub_id, params).await,

        // Directory operations
        "dir.list" => ops::dir::list(sub_id, params).await,
        "dir.create" => ops::dir::create(sub_id, params).await,
        "dir.remove" => ops::dir::remove(sub_id, params).await,

        // Process operations
        "process.run" => ops::process::run(sub_id, params).await,
        "process.start" => ops::process::start(sub_id, params, process_table).await,
        "process.read" => ops::process::read(sub_id, params, process_table).await,
        "process.write" => ops::process::write(sub_id, params, process_table).await,
        "process.kill" => ops::process::kill(sub_id, params, process_table).await,

        // System operations
        "system.info" => ops::system::info(sub_id, params).await,
        "system.getenv" => ops::system::getenv(sub_id, params).await,
        "system.statvfs" => ops::system::statvfs(sub_id, params).await,

        // Watch operations
        "watch.add" => ops::watch::add(sub_id, params, watch_state).await,
        "watch.remove" => ops::watch::remove(sub_id, params, watch_state).await,
        "watch.list" => ops::watch::list(sub_id, params, watch_state).await,

        // Nested batch is not allowed to prevent unbounded recursion.
        "batch" => Response::err(
            sub_id,
            error_code::INVALID_PARAMS,
            "nested batch requests are not allowed",
        ),

        _ => Response::err(
            sub_id,
            error_code::METHOD_NOT_FOUND,
            format!("unknown method: {method}"),
        ),
    }
}

/// Convert a [`Response`] into a MsgPack value suitable for inclusion in a
/// batch result array.
///
/// Each entry in the results array is a map:
/// - On success: `{ result: <value> }`
/// - On error: `{ error: { code: <i32>, message: <string> } }`
fn response_to_value(resp: Response) -> Value {
    if let Some(err) = resp.error {
        Value::Map(vec![(
            Value::String("error".into()),
            Value::Map(vec![
                (
                    Value::String("code".into()),
                    Value::Integer(err.code.into()),
                ),
                (
                    Value::String("message".into()),
                    Value::String(err.message.into()),
                ),
            ]),
        )])
    } else {
        Value::Map(vec![(
            Value::String("result".into()),
            resp.result.unwrap_or(Value::Nil),
        )])
    }
}

// ---------------------------------------------------------------------------
// RPC method handler
// ---------------------------------------------------------------------------

/// `batch` — execute multiple operations in a single round-trip.
///
/// Params:
/// - `requests`: array of sub-request objects, each with:
///   - `method`: RPC method name (string, required)
///   - `params`: method parameters (map, optional, defaults to `{}`)
/// - `parallel`: if `true`, execute sub-requests concurrently (boolean,
///   optional, defaults to `false`)
///
/// Result: `{ results: [ { result: ... } | { error: { code, message } }, … ] }`
///
/// The results array is in the same order as the requests array.
pub async fn batch(
    id: u64,
    params: &Value,
    process_table: Arc<ProcessTable>,
    watch_state: Arc<WatchState>,
) -> Response {
    let requests = match get_array_param(params, "requests") {
        Some(r) => r,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: requests (expected array)",
            );
        }
    };

    if requests.is_empty() {
        return Response::ok(
            id,
            Value::Map(vec![(
                Value::String("results".into()),
                Value::Array(vec![]),
            )]),
        );
    }

    let parallel = get_bool_param(params, "parallel").unwrap_or(false);

    let results = if parallel {
        // Spawn all sub-requests concurrently and collect results in order.
        let mut handles = Vec::with_capacity(requests.len());

        for (idx, req) in requests.iter().enumerate() {
            let method = get_str_field(req, "method").unwrap_or("").to_owned();
            let sub_params = get_params_field(req);
            let sub_id = get_u64_field(req, "id").unwrap_or(idx as u64);
            let pt = Arc::clone(&process_table);
            let ws = Arc::clone(&watch_state);

            handles.push(tokio::spawn(async move {
                dispatch_one(&method, &sub_params, sub_id, &pt, &ws).await
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            let resp = match handle.await {
                Ok(r) => r,
                Err(e) => Response::err(
                    0,
                    error_code::INTERNAL_ERROR,
                    format!("task join error: {e}"),
                ),
            };
            results.push(response_to_value(resp));
        }
        results
    } else {
        // Execute sub-requests sequentially.
        let mut results = Vec::with_capacity(requests.len());

        for (idx, req) in requests.iter().enumerate() {
            let method = get_str_field(req, "method").unwrap_or("");
            let sub_params = get_params_field(req);
            let sub_id = get_u64_field(req, "id").unwrap_or(idx as u64);

            let resp =
                dispatch_one(method, &sub_params, sub_id, &process_table, &watch_state).await;
            results.push(response_to_value(resp));
        }
        results
    };

    Response::ok(
        id,
        Value::Map(vec![(
            Value::String("results".into()),
            Value::Array(results),
        )]),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    fn make_sub_request(method: &str, params: Value) -> Value {
        Value::Map(vec![
            (Value::String("method".into()), Value::String(method.into())),
            (Value::String("params".into()), params),
        ])
    }

    fn pt() -> Arc<ProcessTable> {
        Arc::new(ProcessTable::new())
    }

    fn ws() -> Arc<WatchState> {
        Arc::new(WatchState::new())
    }

    /// Helper to extract the results array from a batch response.
    fn extract_results(resp: &Response) -> &Vec<Value> {
        resp.result
            .as_ref()
            .unwrap()
            .as_map()
            .unwrap()
            .iter()
            .find(|(k, _)| k.as_str() == Some("results"))
            .unwrap()
            .1
            .as_array()
            .unwrap()
    }

    #[tokio::test]
    async fn batch_empty_requests() {
        let params = make_params(vec![("requests", Value::Array(vec![]))]);
        let resp = batch(1, &params, pt(), ws()).await;
        assert!(resp.error.is_none());

        let results = extract_results(&resp);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_missing_requests_param() {
        let params = Value::Map(vec![]);
        let resp = batch(2, &params, pt(), ws()).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn batch_sequential_file_stats() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        std::fs::write(&file_a, b"aaa").unwrap();
        std::fs::write(&file_b, b"bbbbb").unwrap();

        let params = make_params(vec![(
            "requests",
            Value::Array(vec![
                make_sub_request(
                    "file.stat",
                    make_params(vec![(
                        "path",
                        Value::String(file_a.to_str().unwrap().into()),
                    )]),
                ),
                make_sub_request(
                    "file.stat",
                    make_params(vec![(
                        "path",
                        Value::String(file_b.to_str().unwrap().into()),
                    )]),
                ),
            ]),
        )]);

        let resp = batch(3, &params, pt(), ws()).await;
        assert!(resp.error.is_none(), "batch failed: {:?}", resp.error);

        let results = extract_results(&resp);
        assert_eq!(results.len(), 2);

        // Both should have "result" (not "error").
        for entry in results {
            let map = entry.as_map().unwrap();
            assert!(
                map.iter().any(|(k, _)| k.as_str() == Some("result")),
                "expected result field in: {:?}",
                entry
            );
        }
    }

    #[tokio::test]
    async fn batch_parallel_file_stats() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("pa.txt");
        let file_b = dir.path().join("pb.txt");
        std::fs::write(&file_a, b"aa").unwrap();
        std::fs::write(&file_b, b"bb").unwrap();

        let params = make_params(vec![
            (
                "requests",
                Value::Array(vec![
                    make_sub_request(
                        "file.stat",
                        make_params(vec![(
                            "path",
                            Value::String(file_a.to_str().unwrap().into()),
                        )]),
                    ),
                    make_sub_request(
                        "file.stat",
                        make_params(vec![(
                            "path",
                            Value::String(file_b.to_str().unwrap().into()),
                        )]),
                    ),
                ]),
            ),
            ("parallel", Value::Boolean(true)),
        ]);

        let resp = batch(4, &params, pt(), ws()).await;
        assert!(
            resp.error.is_none(),
            "parallel batch failed: {:?}",
            resp.error
        );

        let results = extract_results(&resp);
        assert_eq!(results.len(), 2);

        for entry in results {
            let map = entry.as_map().unwrap();
            assert!(map.iter().any(|(k, _)| k.as_str() == Some("result")));
        }
    }

    #[tokio::test]
    async fn batch_mixed_success_and_error() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("exists.txt");
        std::fs::write(&file, b"data").unwrap();

        let params = make_params(vec![(
            "requests",
            Value::Array(vec![
                make_sub_request(
                    "file.stat",
                    make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]),
                ),
                make_sub_request(
                    "file.stat",
                    make_params(vec![(
                        "path",
                        Value::String("/tmp/__batch_nonexistent_9876__".into()),
                    )]),
                ),
            ]),
        )]);

        let resp = batch(5, &params, pt(), ws()).await;
        assert!(resp.error.is_none());

        let results = extract_results(&resp);
        assert_eq!(results.len(), 2);

        // First should succeed.
        let first = results[0].as_map().unwrap();
        assert!(
            first.iter().any(|(k, _)| k.as_str() == Some("result")),
            "first sub-request should succeed"
        );

        // Second should fail.
        let second = results[1].as_map().unwrap();
        assert!(
            second.iter().any(|(k, _)| k.as_str() == Some("error")),
            "second sub-request should fail"
        );
    }

    #[tokio::test]
    async fn batch_unknown_method() {
        let params = make_params(vec![(
            "requests",
            Value::Array(vec![make_sub_request(
                "nonexistent.method",
                Value::Map(vec![]),
            )]),
        )]);

        let resp = batch(6, &params, pt(), ws()).await;
        assert!(resp.error.is_none()); // batch itself succeeds

        let results = extract_results(&resp);
        assert_eq!(results.len(), 1);

        let entry = results[0].as_map().unwrap();
        let err = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("error"))
            .unwrap()
            .1
            .as_map()
            .unwrap();
        let code = err
            .iter()
            .find(|(k, _)| k.as_str() == Some("code"))
            .unwrap()
            .1
            .as_i64()
            .unwrap();
        assert_eq!(code, error_code::METHOD_NOT_FOUND as i64);
    }

    #[tokio::test]
    async fn batch_nested_batch_rejected() {
        let params = make_params(vec![(
            "requests",
            Value::Array(vec![make_sub_request(
                "batch",
                make_params(vec![("requests", Value::Array(vec![]))]),
            )]),
        )]);

        let resp = batch(7, &params, pt(), ws()).await;
        assert!(resp.error.is_none()); // outer batch succeeds

        let results = extract_results(&resp);
        assert_eq!(results.len(), 1);

        let entry = results[0].as_map().unwrap();
        assert!(
            entry.iter().any(|(k, _)| k.as_str() == Some("error")),
            "nested batch should be rejected"
        );
    }

    #[tokio::test]
    async fn batch_missing_method_in_sub_request() {
        let params = make_params(vec![(
            "requests",
            Value::Array(vec![
                // Sub-request without "method" field — uses empty string default.
                Value::Map(vec![(Value::String("params".into()), Value::Map(vec![]))]),
            ]),
        )]);

        let resp = batch(8, &params, pt(), ws()).await;
        assert!(resp.error.is_none());

        let results = extract_results(&resp);
        assert_eq!(results.len(), 1);

        // Should get a METHOD_NOT_FOUND error for the empty method string.
        let entry = results[0].as_map().unwrap();
        let err = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("error"))
            .unwrap()
            .1
            .as_map()
            .unwrap();
        let code = err
            .iter()
            .find(|(k, _)| k.as_str() == Some("code"))
            .unwrap()
            .1
            .as_i64()
            .unwrap();
        assert_eq!(code, error_code::METHOD_NOT_FOUND as i64);
    }

    #[tokio::test]
    async fn batch_process_run_in_batch() {
        let params = make_params(vec![(
            "requests",
            Value::Array(vec![make_sub_request(
                "process.run",
                make_params(vec![
                    ("program", Value::String("echo".into())),
                    (
                        "args",
                        Value::Array(vec![Value::String("batch_echo".into())]),
                    ),
                ]),
            )]),
        )]);

        let resp = batch(9, &params, pt(), ws()).await;
        assert!(resp.error.is_none());

        let results = extract_results(&resp);
        assert_eq!(results.len(), 1);

        let entry = results[0].as_map().unwrap();
        let result = entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("result"))
            .unwrap()
            .1
            .as_map()
            .unwrap();

        let stdout = result
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(stdout).trim(), "batch_echo");
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let dir = TempDir::new().unwrap();

        // Create files with different sizes so we can verify ordering.
        for i in 0..5 {
            let file = dir.path().join(format!("order_{i}.txt"));
            std::fs::write(&file, "x".repeat(i + 1)).unwrap();
        }

        let requests: Vec<Value> = (0..5)
            .map(|i| {
                let file = dir.path().join(format!("order_{i}.txt"));
                make_sub_request(
                    "file.stat",
                    make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]),
                )
            })
            .collect();

        let params = make_params(vec![("requests", Value::Array(requests))]);
        let resp = batch(10, &params, pt(), ws()).await;
        assert!(resp.error.is_none());

        let results = extract_results(&resp);
        assert_eq!(results.len(), 5);

        // Verify sizes are in order: 1, 2, 3, 4, 5.
        for (i, entry) in results.iter().enumerate() {
            let result = entry
                .as_map()
                .unwrap()
                .iter()
                .find(|(k, _)| k.as_str() == Some("result"))
                .unwrap()
                .1
                .as_map()
                .unwrap();
            let size = result
                .iter()
                .find(|(k, _)| k.as_str() == Some("size"))
                .unwrap()
                .1
                .as_u64()
                .unwrap();
            assert_eq!(size, (i + 1) as u64, "entry {i} has wrong size");
        }
    }
}
