//! IPC round-trip benchmark.
//!
//! Target: p99 < 0.5 ms inside Flatpak (PLAN.md §6 Phase 2 exit gate).

use criterion::{Criterion, criterion_group, criterion_main};
use std::time::Duration;
use tempfile::tempdir;

use compass_ipc::{Client, Listener, Request, Response, SocketPath};
use tokio::runtime::Runtime;

fn run_ping_once() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let dir = tempdir().unwrap();
        let socket_path = SocketPath::in_dir(dir.path());

        let listener = Listener::bind(socket_path.as_path()).await.unwrap();
        let server_handle = tokio::spawn(async move {
            listener
                .serve(|request| async move {
                    match request {
                        Request::Ping => Response::Pong {
                            protocol_version: 1,
                            pid: std::process::id(),
                        },
                        Request::Query { text: _ } => Response::QueryResults { hits: vec![] },
                        _ => Response::Ack,
                    }
                })
                .await
                .unwrap();
        });

        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut client = Client::connect(socket_path.as_path()).await.unwrap();
        let response = client.request(Request::Ping).await.unwrap();

        drop(client);
        server_handle.abort();

        criterion::black_box(response);
    });
}

fn run_throughput(concurrent: usize) {
    let rt = Runtime::new().unwrap();
    rt.block_on(async move {
        let dir = tempdir().unwrap();
        let socket_path = SocketPath::in_dir(dir.path());

        let listener = Listener::bind(socket_path.as_path()).await.unwrap();
        let server_handle = tokio::spawn(async move {
            listener
                .serve(|request| async move {
                    match request {
                        Request::Ping => Response::Pong {
                            protocol_version: 1,
                            pid: std::process::id(),
                        },
                        _ => Response::Ack,
                    }
                })
                .await
                .unwrap();
        });

        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut handles = vec![];
        for _ in 0..concurrent {
            let socket_path = socket_path.clone();
            let handle = tokio::spawn(async move {
                let mut client = Client::connect(socket_path.as_path()).await.unwrap();
                let _ = client.request(Request::Ping).await.unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        server_handle.abort();
    });
}

fn bench_ping(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_roundtrip");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(1000);

    group.bench_function("ping", |b| {
        b.iter(run_ping_once);
    });
    group.finish();
}

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_throughput");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(100);

    for concurrent in [1, 10, 50, 100] {
        group.bench_with_input(
            format!("concurrent_{}", concurrent),
            &concurrent,
            |b, &concurrent| {
                b.iter_batched(
                    || concurrent,
                    run_throughput,
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_ping, bench_throughput);
criterion_main!(benches);
