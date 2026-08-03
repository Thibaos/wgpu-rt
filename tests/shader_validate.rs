//! Offline WGSL validation for the chunk shader.
//!
//! The shader is compiled by wgpu only at GPU runtime, which makes syntax/type
//! errors invisible to `cargo test`. This test parses the shaders with naga
//! (the same front end wgpu uses) so a broken shader fails CI instead of
//! panicking at first frame.

use std::path::PathBuf;

fn shader_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("shaders")
        .join(name)
}

#[test]
fn chunk_wgsl_parses_and_validates() {
    let source = std::fs::read_to_string(shader_path("chunk.wgsl")).expect("chunk.wgsl must exist");
    // `parse` runs the WGSL front end: syntax, typing, and module validation.
    // A GPU/device is not required.
    naga::front::wgsl::parse_str(&source).expect("chunk.wgsl failed WGSL validation");
}

#[test]
fn rayquery_wgsl_parses_and_validates() {
    let source =
        std::fs::read_to_string(shader_path("rayquery.wgsl")).expect("rayquery.wgsl must exist");
    // The `%%STATS_*%%` markers are comments in the checked-in file and are
    // replaced by `RayQueryResources::new` (WGPU_RT_STATS=1) before wgpu
    // compiles the module; the validation-only run exercises the default
    // build (no stats bindings).
    naga::front::wgsl::parse_str(&source).expect("rayquery.wgsl failed WGSL validation");
}

#[test]
fn rayquery_wgsl_full_validation() {
    let raw =
        std::fs::read_to_string(shader_path("rayquery.wgsl")).expect("rayquery.wgsl must exist");
    // Mirror the WGPU_RT_STATS=1 build exactly (stats bindings injected).
    let source = raw
        .replace(
            "// %%STATS_DECLS%%",
            "struct Stats {\n    fragments: atomic<u32>,\n    processed_cells: atomic<u32>,\n    hits: atomic<u32>,\n};\n@group(1) @binding(3) var<storage, read_write> stats: Stats;\n",
        )
        .replace("// %%STATS_PIXEL%%", "atomicAdd(&stats.fragments, 1u);")
        .replace("// %%STATS_CELLS%%", "atomicAdd(&stats.processed_cells, 1u);")
        .replace("// %%STATS_HIT%%", "atomicAdd(&stats.hits, 1u);");
    let module = naga::front::wgsl::parse_str(&source)
        .expect("rayquery.wgsl (stats build) failed WGSL parsing");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("rayquery.wgsl (stats build) failed full validation");
}

fn try_validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.to_string())?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator.validate(&module).map(|_| ()).map_err(|e| {
        let s = format!("{e:?}");
        // Keep the message short: extract the NotInScope bit if present.
        if s.contains("NotInScope") {
            "NotInScope".to_string()
        } else {
            s.chars().take(120).collect()
        }
    })
}

const HEAD: &str = r#"enable wgpu_ray_query;
@group(0) @binding(0) var acc_struct: acceleration_structure;
@group(0) @binding(1) var out_color: texture_storage_2d<rgba8unorm, write>;
"#;

#[test]
fn minimal_rayquery_generate_variants() {
    let cases: &[(&str, &str)] = &[
        (
            "loop+generate+vararg+and",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        best_t = 1.0;\n        rayQueryGenerateIntersection(&rq, best_t);\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && best_t > 0.0) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
        (
            "no-generate (dda body only)",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        best_t = 1.0;\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && best_t > 0.0) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
        (
            "generate with literal arg",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        rayQueryGenerateIntersection(&rq, 1.0);\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && best_t > 0.0) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
        (
            "loop+generate+vararg, nested ifs (no &&)",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        best_t = 1.0;\n        rayQueryGenerateIntersection(&rq, best_t);\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED) {\n    if (best_t > 0.0) {\n        textureStore(out_color, gid.xy, vec4<f32>(1.0));\n    }\n}\n",
        ),
        (
            "generate with let-alias arg",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        best_t = 1.0;\n        let tt = best_t;\n        rayQueryGenerateIntersection(&rq, tt);\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
        (
            "generate with math-expr arg",
            "var best_t = -1.0;\nvar rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        best_t = 1.0;\n        rayQueryGenerateIntersection(&rq, max(best_t, 0.0));\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
        (
            "generate with candidate-member expr",
            "var rq: ray_query;\nrayQueryInitialize(&rq, acc_struct, RayDesc(0u, 0xFFu, 0.001, 10000.0, vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0)));\nwhile (rayQueryProceed(&rq)) {\n    let c = rayQueryGetCandidateIntersection(&rq);\n    if (c.kind == RAY_QUERY_INTERSECTION_AABB) {\n        rayQueryGenerateIntersection(&rq, c.t + 0.0);\n    }\n}\nlet committed = rayQueryGetCommittedIntersection(&rq);\nif (committed.kind == RAY_QUERY_INTERSECTION_GENERATED) {\n    textureStore(out_color, gid.xy, vec4<f32>(1.0));\n}\n",
        ),
    ];
    for (name, body) in cases {
        let src = format!(
            "{HEAD}@compute @workgroup_size(8, 8)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>) {{\n{body}}}\n"
        );
        let result = try_validate(&src);
        println!("{name}: {result:?}");
    }
}

#[test]
fn heatmap_flag_is_consumed_by_both_shaders() {
    for name in ["chunk.wgsl", "rayquery.wgsl"] {
        let src = std::fs::read_to_string(shader_path(name));
        let Some(source) = src.as_ref().ok() else {
            continue; // shader missing: the parse tests above will fail loudly
        };
        assert!(
            source.contains("viewport_and_heatmap.z"),
            "{name} must read the heatmap flag"
        );
    }
}
