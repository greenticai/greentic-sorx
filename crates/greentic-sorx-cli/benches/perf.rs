use criterion::{Criterion, criterion_group, criterion_main};

fn bench_cli_parse_start(c: &mut Criterion) {
    c.bench_function("cli_parse_start_schema", |b| {
        b.iter(|| {
            let cli = greentic_sorx_cli::parse_from([
                "greentic-sorx",
                "start",
                "landlord.gtpack",
                "--schema",
            ])
            .expect("start schema command should parse");
            std::hint::black_box(cli);
        })
    });
}

criterion_group!(benches, bench_cli_parse_start);
criterion_main!(benches);
