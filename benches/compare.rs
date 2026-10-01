use core::hint::black_box;

use codspeed_criterion_compat::{Criterion, criterion_group, criterion_main};
use idn::Config;
use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

fn to_ascii(c: &mut Criterion) {
    let mut group = c.benchmark_group("to_ascii");
    let uts46 = Uts46::new();
    let config = Config::new();
    for (name, input) in INPUTS {
        group.bench_function(format!("idn/{name}"), |b| {
            b.iter(|| config.to_ascii(black_box(input)).unwrap())
        });
        group.bench_function(format!("idna/{name}"), |b| {
            b.iter(|| {
                uts46
                    .to_ascii(
                        black_box(input.as_bytes()),
                        AsciiDenyList::EMPTY,
                        Hyphens::Allow,
                        DnsLength::Ignore,
                    )
                    .unwrap()
            })
        });
        group.bench_function(format!("idna-0.5/{name}"), |b| {
            b.iter(|| idna_0_5::domain_to_ascii(black_box(input)).unwrap())
        });
    }
}

fn to_unicode(c: &mut Criterion) {
    let mut group = c.benchmark_group("to_unicode");
    let uts46 = Uts46::new();
    let config = Config::new();
    for (name, input) in INPUTS {
        group.bench_function(format!("idn/{name}"), |b| {
            b.iter(|| config.to_unicode(black_box(input)).unwrap())
        });
        group.bench_function(format!("idna/{name}"), |b| {
            b.iter(|| {
                let (output, result) = uts46.to_unicode(
                    black_box(input.as_bytes()),
                    AsciiDenyList::EMPTY,
                    Hyphens::Allow,
                );
                result.unwrap();
                output
            })
        });
        group.bench_function(format!("idna-0.5/{name}"), |b| {
            b.iter(|| {
                let (output, result) = idna_0_5::domain_to_unicode(black_box(input));
                result.unwrap();
                output
            })
        });
    }
}

const INPUTS: &[(&str, &str)] = &[
    ("ascii", "www.example.com"),
    ("ascii_upper", "WWW.Example.COM"),
    ("punycode", "xn--bcher-kva.example"),
    ("latin", "bücher.example"),
    ("latin_upper", "BÜCHER.example"),
    ("cjk", "日本語。ＪＰ"),
    ("arabic", "مثال.إختبار"),
    ("greek", "δοκιμή.ελ"),
    ("devanagari", "उदाहरण.परीक्षा"),
    ("punycode_cjk", "xn--wgv71a119e.jp"),
];

criterion_group!(benches, to_ascii, to_unicode);
criterion_main!(benches);
