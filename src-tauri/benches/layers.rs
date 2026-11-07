//! 分层性能基准（scripts/bench 持续方案，2026-11-07 立项）。
//!
//! 只测确定性纯层：matching 打分、vault 库读写。临时库自建（路径含进程号，
//! 绝不碰用户真实 %APPDATA%），DPAPI 加密路径与真实链路同库同码。
//! SearchEngine 全链路不进这里——它装配真实开始菜单/Everything，数字随机器
//! 漂移；端到端数字走 CDP 实测层（release 构建 + 同机基线）。
//!
//! 运行：cargo bench --bench layers（数字随机器变，只与本机 baseline.json 比）

use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use zap_lib::providers::db;
use zap_lib::providers::matching::MatchForms;
use zap_lib::providers::vault;

/// matching 语料：中文全拼 / 首字母 / 英文前缀 / 中英混合 / 未命中，
/// 覆盖 matching.rs 三级匹配（直连 → 拼音全拼 → 拼音首字母）
const MATCH_CORPUS: &[(&str, &str)] = &[
    ("微信", "wx"),
    ("微信", "weixin"),
    ("记事本", "jishiben"),
    ("远程桌面连接", "yuancheng"),
    ("Visual Studio Code", "vsc"),
    ("Microsoft Edge", "edge"),
    ("腾讯会议", "txhy"),
    ("网易云音乐", "abc"), // 未命中
];

fn bench_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("matching");
    for (hay, query) in MATCH_CORPUS {
        group.bench_with_input(
            BenchmarkId::new("forms_score", format!("{hay}/{query}")),
            &(*hay, *query),
            |b, (hay, query)| {
                b.iter(|| {
                    let forms = MatchForms::new(black_box(hay));
                    black_box(forms.score(black_box(query)))
                });
            },
        );
    }
    group.finish();
}

/// vault 夹具：临时目录 + 50 条账号（含 1 条 TOTP），基准进程内建一次；
/// 循环内只跑查询路径，写入成本不在测量内
fn bench_vault(c: &mut Criterion) {
    let dir: PathBuf = std::env::temp_dir().join(format!("zap-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录失败");
    let db_path = dir.join("bench.db");
    let _ = std::fs::remove_file(&db_path);
    let conn = db::open_db(&db_path).expect("开临时库失败");
    for i in 0..50 {
        vault::add(
            &conn,
            &format!("基准账号{i:02}"),
            &format!("user{i}@example.com"),
            &format!("pw-{i}-secret"),
            &format!("https://site{i}.example.com"),
            "",
            if i == 0 { "JBSWY3DPEHPK3PXP" } else { "" },
        )
        .expect("夹具写入失败");
    }
    // 详情页解密路径（DPAPI）抽一条代表真实读密成本
    let detail_id = vault::search(&conn, "")
        .iter()
        .find_map(|it| it.target.parse::<i64>().ok())
        .expect("夹具查询失败");

    let mut group = c.benchmark_group("vault");
    group.bench_function("search/空词全量50条", |b| {
        b.iter(|| black_box(vault::search(black_box(&conn), black_box(""))))
    });
    group.bench_function("search/拼音词", |b| {
        b.iter(|| black_box(vault::search(black_box(&conn), black_box("jizhun"))))
    });
    group.bench_function("get/解密详情", |b| {
        b.iter(|| black_box(vault::get(black_box(&conn), detail_id)))
    });
    group.finish();

    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, bench_matching, bench_vault);
criterion_main!(benches);
