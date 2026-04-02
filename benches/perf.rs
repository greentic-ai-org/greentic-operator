use criterion::{Criterion, criterion_group, criterion_main};
use greentic_operator::demo::card::detect_adaptive_card_view;
use greentic_operator::demo::http_ingress::benchmark_parse_ingress_route;
use greentic_operator::demo::runner_host::benchmark_extract_token_validation_request;
use greentic_operator::operator_i18n;
use greentic_operator::static_routes::{
    ActiveRouteTable, CacheStrategy, RouteScopeSegment, StaticRouteDescriptor,
};
use serde_json::json;
use std::path::PathBuf;

fn benchmark_i18n_hot_path(c: &mut Criterion) {
    c.bench_function("operator_i18n::tr_for_locale", |b| {
        b.iter(|| {
            let rendered = operator_i18n::tr_for_locale(
                "cli.main.help.tagline",
                "Greentic operator tooling",
                "en-US",
            );
            criterion::black_box(rendered);
        })
    });
}

fn benchmark_card_parse(c: &mut Criterion) {
    let payload = json!({
        "payload": {
            "outputs": {
                "card": {
                    "type": "AdaptiveCard",
                    "version": "1.4",
                    "summary": "Please confirm",
                    "body": [
                        {"type": "TextBlock", "text": "Hello"},
                        {
                            "type": "Container",
                            "items": [
                                {"type": "Input.Text", "id": "comment", "placeholder": "Add a comment"},
                                {
                                    "type": "ColumnSet",
                                    "columns": [
                                        {
                                            "items": [
                                                {"type": "Input.Toggle", "id": "opt_in", "title": "Opt in"}
                                            ]
                                        }
                                    ]
                                }
                            ]
                        }
                    ],
                    "actions": [
                        {"type": "Action.Submit", "title": "Submit", "id": "submit"},
                        {"type": "Action.ShowCard", "title": "More", "actionId": "more-info"}
                    ]
                }
            }
        }
    });

    c.bench_function("demo::card::parse::detect_adaptive_card_view", |b| {
        b.iter(|| {
            let parsed = detect_adaptive_card_view(criterion::black_box(&payload));
            criterion::black_box(parsed);
        })
    });
}

fn benchmark_route_match(c: &mut Criterion) {
    let table = ActiveRouteTable::from_plan(&greentic_operator::static_routes::StaticRoutePlan {
        routes: vec![
            StaticRouteDescriptor {
                route_id: "docs".to_string(),
                pack_id: "pack.docs".to_string(),
                pack_path: PathBuf::from("/tmp/docs.gtpack"),
                public_path: "/static/docs".to_string(),
                source_root: "site".to_string(),
                index_file: Some("index.html".to_string()),
                spa_fallback: Some("index.html".to_string()),
                tenant_scoped: false,
                team_scoped: false,
                cache_strategy: CacheStrategy::PublicMaxAge {
                    max_age_seconds: 300,
                },
                route_segments: vec![
                    RouteScopeSegment::Literal("static".to_string()),
                    RouteScopeSegment::Literal("docs".to_string()),
                ],
            },
            StaticRouteDescriptor {
                route_id: "tenant-app".to_string(),
                pack_id: "pack.app".to_string(),
                pack_path: PathBuf::from("/tmp/app.gtpack"),
                public_path: "/apps/tenant".to_string(),
                source_root: "ui".to_string(),
                index_file: Some("index.html".to_string()),
                spa_fallback: Some("index.html".to_string()),
                tenant_scoped: true,
                team_scoped: true,
                cache_strategy: CacheStrategy::None,
                route_segments: vec![
                    RouteScopeSegment::Literal("apps".to_string()),
                    RouteScopeSegment::Tenant,
                    RouteScopeSegment::Team,
                ],
            },
        ],
        warnings: Vec::new(),
        blocking_failures: Vec::new(),
    });

    c.bench_function("static_routes::ActiveRouteTable::match_request", |b| {
        b.iter(|| {
            let matched = table.match_request(criterion::black_box(
                "/apps/acme/default/dashboard/index.html",
            ));
            criterion::black_box(matched);
        })
    });
}

fn benchmark_http_ingress_route_parse(c: &mut Criterion) {
    let path = "/v1/messaging/ingress/provider-a/tenant-x/team-y/handler-z";
    c.bench_function("demo::http_ingress::parse_route_segments", |b| {
        b.iter(|| {
            let parsed = benchmark_parse_ingress_route(criterion::black_box(path));
            criterion::black_box(parsed);
        })
    });
}

fn benchmark_runner_host_token_request(c: &mut Criterion) {
    let payload = serde_json::to_vec(&json!({
        "headers": {
            "Authorization": "Bearer token-123"
        },
        "token_validation": {
            "issuer": "https://issuer.example",
            "audience": ["api://svc", "api://secondary"],
            "required_scopes": "read write admin"
        },
        "metadata": {
            "authorization": "Bearer ignored-because-headers-win"
        }
    }))
    .expect("payload bytes");

    c.bench_function("demo::runner_host::extract_token_validation_request", |b| {
        b.iter(|| {
            let parsed = benchmark_extract_token_validation_request(criterion::black_box(&payload));
            criterion::black_box(parsed);
        })
    });
}

criterion_group!(
    benches,
    benchmark_i18n_hot_path,
    benchmark_card_parse,
    benchmark_route_match,
    benchmark_http_ingress_route_parse,
    benchmark_runner_host_token_request
);
criterion_main!(benches);
