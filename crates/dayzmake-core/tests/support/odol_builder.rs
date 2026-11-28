//! Test helper: a synthetic ODOL v56 model written byte by byte from `docs/superpowers/specs/2026-11-16-odol-v56-layout.md`.
//!
//! The model is hand made: two LODs, a model-level skeleton and animations, and in the first LOD a proxy, two
//! textures, a material with one stage, a section, a named selection and a property. The second LOD holds a
//! named selection whose index arrays are big enough (1200 bytes) to be stored LZO compressed.
//! A few knobs let a test corrupt one thing at a time.

#![expect(
    clippy::similar_names,
    reason = "test helper building a small model from constants"
)]

/// A size or offset of the small synthetic model as `u32`; the model is far below the limit.
fn u32_of(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A small number as `u16`; the callers pass values below 4.
fn u16_of(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// Number of `u16` entries of the compressed arrays in LOD 1 (1200 bytes, above the 1024 byte threshold).
pub const BIG_ARRAY: usize = 600;

/// What to corrupt in an otherwise valid model.
#[derive(Default, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches of a test builder"
)]
pub struct Knobs {
    /// Writes this `lod_count` (the arrays that follow keep two entries).
    pub lod_count: Option<u32>,
    /// Writes this as the start of LOD 1 instead of its real offset.
    pub lod1_start: Option<u32>,
    /// Leaves the NUL of the property class string out (the file ends in that string).
    pub unterminated_property_class: bool,
    /// Declares the big array in LOD 1 with this many elements and no bytes at all.
    pub huge_array: Option<u32>,
    /// The LZO stream of the big array in LOD 1 ends after half of the declared size.
    pub lzo_short: bool,
    /// The LZO stream of the big array in LOD 1 decompresses to twice the declared size.
    pub lzo_overrun: bool,
    /// The stage texture of the material in LOD 0 is the same file as a texture of the LOD.
    pub repeat_texture: bool,
    /// The selection name in LOD 0 contains an ESC control character.
    pub control_char_name: bool,
    /// The material in LOD 0 has an empty name (a built-in material).
    pub empty_material_name: bool,
    /// The material in LOD 0 is listed twice.
    pub repeat_material: bool,
    /// The model has no skeleton (an empty name, no bones).
    pub no_skeleton: bool,
    /// The property class and damage strings are empty.
    pub empty_class_damage: bool,
    /// No animations, and the LODs have no proxies, textures, materials, selections or properties.
    pub bare: bool,
    /// Writes this many LODs, all copies of LOD 0 one after the other, each declaring its end as the end of the file.
    pub repeat_lod0_to_eof: Option<usize>,
}

/// The little endian writer of the model.
#[derive(Default)]
pub struct Out(pub Vec<u8>);

impl Out {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    pub fn zeros(&mut self, n: usize) {
        self.0.resize(self.0.len() + n, 0);
    }
    pub fn cstr(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.u8(0);
    }
    pub fn vec3(&mut self, v: [f32; 3]) {
        for c in v {
            self.f32(c);
        }
    }
    pub fn matrix(&mut self) {
        // identity orientation, position (1, 2, 3)
        for v in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0] {
            self.f32(v);
        }
    }
    pub fn pos(&self) -> usize {
        self.0.len()
    }
    pub fn patch_u32(&mut self, at: usize, v: u32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A compressed array: the element count is written by the caller, this writes the bytes. They are LZO
    /// compressed (with the end marker, no length) when there are at least 1024 of them.
    pub fn compressed(&mut self, data: &[u8]) {
        if data.len() >= 1024 {
            let stream = lzo1x::compress(data, lzo1x::CompressLevel::default());
            self.bytes(&stream);
        } else {
            self.bytes(data);
        }
    }
    /// `u32 count` + a compressed array of `u16`.
    pub fn index_array(&mut self, values: &[u16]) {
        self.u32(u32_of(values.len()));
        let mut raw = Vec::new();
        for v in values {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        self.compressed(&raw);
    }
}

fn model_info(o: &mut Out, knobs: Knobs) {
    o.u32(0); // index
    o.f32(1.0); // mem_lod_sphere
    o.f32(1.0); // geo_lod_sphere
    o.u32(0); // remarks
    o.u32(0); // and_hints
    o.u32(0); // or_hints
    o.vec3([0.0; 3]); // aiming_center
    o.u32(0xff00_0000); // map_icon_color
    o.u32(0xff00_0000); // map_selected_color
    o.f32(-10.0); // view_density
    o.vec3([-1.0, -2.0, -3.0]); // bbox_min
    o.vec3([1.0, 2.0, 3.0]); // bbox_max
    o.vec3([-1.0, -2.0, -3.0]); // bbox_visual_min
    o.vec3([1.0, 2.0, 3.0]); // bbox_visual_max
    o.vec3([0.0; 3]); // bounding_center
    o.vec3([0.0; 3]); // geometry_center
    o.vec3([0.0; 3]); // center_of_mass
    o.zeros(36); // inverse_inertia
    for b in [1, 0, 1, 1, 1] {
        o.u8(b); // auto_center, lock_auto_center, can_occlude, can_be_occluded, (unknown)
    }
    for f in [1.0f32, 0.0, 60.0, 1800.0, 30.0, 0.0, 1.0, 37.0] {
        o.f32(f); // (unknown), (unknown), then the six ht_min .. t_body style values
    }
    o.u8(0); // force_not_alpha
    o.i32(0); // sb_source
    o.u8(0); // prefer_shadow_volume
    o.f32(0.0); // shadow_offset
    o.u8(1); // animated
    // skeleton
    if knobs.no_skeleton {
        o.cstr(""); // no more skeleton fields after an empty name
    } else {
        o.cstr("Skel");
        o.u8(1); // is_discrete
        o.u32(2); // bone_count
        o.cstr("bone_a");
        o.cstr("");
        o.cstr("bone_b");
        o.cstr("bone_a");
        o.cstr(""); // pivots_name
    }
    o.u8(0); // map_type
    o.u32(0); // mass array: count 0
    o.f32(12.5); // mass
    o.f32(0.08); // inv_mass
    o.f32(12.5); // alt_mass
    o.f32(0.08); // inv_alt_mass
    for v in [0xff_u8; 13] {
        o.u8(v); // the LOD indices of the special LODs (memory, geometry, ...)
    }
    o.u32(0); // min_shadow
    o.u8(1); // can_blend
    if knobs.unterminated_property_class {
        o.bytes(b"ClassName");
        return;
    }
    let (class, damage) = if knobs.empty_class_damage {
        ("", "")
    } else {
        ("ClassName", "DamageName")
    };
    o.cstr(class); // property_class
    o.cstr(damage); // property_damage
    o.u8(0); // property_frequent
    o.u32(0); // n
    o.u8(0); // unknown flag
}

fn animations(o: &mut Out, lods: usize) {
    o.u32(2); // class_count
    for (kind, name, source) in [(0u32, "turn", "door"), (9, "vanish", "hide")] {
        o.u32(kind);
        o.cstr(name);
        o.cstr(source);
        for f in [0.0f32, 1.0, 0.0, 1.0] {
            o.f32(f); // min_phase, max_phase, min_value, max_value
        }
        o.u32(0); // source_address
        if kind == 0 {
            o.f32(0.0);
            o.f32(1.5);
        } else {
            o.f32(0.01); // hide_value
        }
    }
    o.i32(i32::try_from(lods).unwrap_or(i32::MAX)); // res_count == lod_count
    for _ in 0..lods {
        // bones to animations: 2 bones; bone 0 has animation 0
        o.u32(2);
        o.u32(1);
        o.u32(0);
        o.u32(0);
    }
    for _ in 0..lods {
        // animations to bones: class 0 (rotation) has axis, class 1 (hide) has not
        o.i32(0);
        o.vec3([0.0; 3]);
        o.vec3([0.0, 1.0, 0.0]);
        o.i32(1);
    }
}

/// LOD 0 without proxies, textures, materials, selections and properties.
fn lod0_bare(o: &mut Out) {
    o.u32(0); // proxies
    o.u32(0); // items
    o.u32(0); // bone links
    common_head(o, 3);
    o.u32(0); // textures
    o.u32(0); // materials
    o.index_array(&[]);
    o.index_array(&[]);
    faces(o, 1);
    o.u32(0); // sections
    o.u32(0); // selections
    o.u32(0); // properties
    tail(o);
}

fn lod0(o: &mut Out, knobs: Knobs) {
    if knobs.bare {
        return lod0_bare(o);
    }
    o.u32(1); // proxy_count
    o.cstr("\\dz\\proxy\\thing.p3d");
    o.matrix();
    for v in [1, 0, -1, 0] {
        o.i32(v); // sequence_id, named_selection_index, bone_index, section_index
    }
    o.u32(0); // item_count
    o.u32(0); // bone_link_count
    common_head(o, 3);
    o.u32(2); // textures
    o.cstr("\\dz\\tex_a_co.paa");
    o.cstr("\\dz\\tex_b_co.paa");
    o.u32(if knobs.repeat_material { 2 } else { 1 }); // materials
    material(o, knobs);
    if knobs.repeat_material {
        material(o, knobs);
    }
    o.index_array(&[]); // edges
    o.index_array(&[]);
    faces(o, 1);
    // one section
    o.u32(1);
    for v in [0u32, 7, 0, 0, 0] {
        o.u32(v);
    }
    o.u16(0); // texture_index
    o.u32(0); // face_flags
    o.i32(0); // material_index
    o.u32(1); // stage_count
    o.f32(0.0);
    // one selection
    o.u32(1);
    o.cstr(if knobs.control_char_name {
        "bo\x1bdy"
    } else {
        "body"
    });
    o.index_array(&[0]);
    o.u32(0);
    o.u8(1); // is_sectional
    o.u32(1);
    o.i32(0);
    o.index_array(&[0, 1, 2]);
    o.u32(3);
    o.bytes(&[255, 255, 255]);
    // one property
    o.u32(1);
    o.cstr("class");
    o.cstr("house");
    tail(o);
}

fn material(o: &mut Out, knobs: Knobs) {
    o.cstr(if knobs.empty_material_name {
        ""
    } else {
        "\\dz\\mat.rvmat"
    });
    o.u32(20); // mat_version
    o.zeros(200); // colors and lighting values
    o.f32(10.0); // specular_power
    for v in [1, 2, 3, 4] {
        o.i32(v); // pixel_shader, vertex_shader, main_light, fog_mode
    }
    o.cstr("\\dz\\surface.bisurf");
    o.u32(1); // render_flag_count
    o.u32(0); // render_flags
    o.u32(1); // stage_count
    o.u32(1); // transform_count
    stage(
        o,
        if knobs.repeat_texture {
            "\\dz\\tex_a_co.paa"
        } else {
            "\\dz\\stage_co.paa"
        },
        0,
    );
    o.u32(0); // uv_source
    o.matrix();
    stage(o, "", 0); // the dummy stage after the transforms
}

fn stage(o: &mut Out, texture: &str, id: u32) {
    o.u32(0); // render_flags
    o.cstr(texture);
    o.u32(id);
    o.u8(0); // use_world_env
}

fn common_head(o: &mut Out, vertex_count: u32) {
    o.u32(vertex_count);
    o.f32(1.0); // face_area
    o.u32(0);
    o.u32(0); // or_hints, and_hints
    o.vec3([-1.0; 3]);
    o.vec3([1.0; 3]);
    o.vec3([0.0; 3]);
    o.f32(2.0); // bradius
}

fn faces(o: &mut Out, count: u32) {
    o.u32(count);
    let bytes = count * 8; // per face: its vertex count + 1 u16, here with 3 indices
    o.u32(bytes);
    o.u16(0);
    for _ in 0..count {
        o.u8(3);
        for i in 0..3u16 {
            o.u16(i);
        }
    }
}

fn tail(o: &mut Out) {
    o.u32(0); // frame_count
    o.zeros(12); // icon_color, selected_color, special
    o.u8(0); // vertex_bone_ref_is_simple
    o.u32(0); // size_of_rest
}

fn lod1(o: &mut Out, knobs: Knobs) {
    o.u32(0); // proxies
    o.u32(0); // items
    o.u32(0); // bone links
    common_head(o, 4);
    o.u32(0); // textures
    o.u32(0); // materials
    o.index_array(&[]);
    o.index_array(&[]);
    faces(o, 2);
    o.u32(0); // sections
    if knobs.bare {
        o.u32(0); // selections
        o.u32(0); // properties
        return tail(o);
    }
    // one selection with big arrays
    o.u32(1);
    o.cstr("big");
    let big: Vec<u16> = (0..BIG_ARRAY).map(|i| u16_of(i % 4)).collect();
    if let Some(count) = knobs.huge_array {
        o.u32(count);
    } else if knobs.lzo_short {
        let mut raw = Vec::new();
        for v in big.iter().take(BIG_ARRAY / 2) {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        let stream = lzo1x::compress(&raw, lzo1x::CompressLevel::default());
        o.u32(u32_of(BIG_ARRAY));
        o.bytes(&stream);
    } else if knobs.lzo_overrun {
        // the declared count is smaller than the stream decompresses to
        let mut raw = Vec::new();
        for v in big.iter().chain(&big) {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        let stream = lzo1x::compress(&raw, lzo1x::CompressLevel::default());
        o.u32(u32_of(BIG_ARRAY));
        o.bytes(&stream);
    } else {
        o.index_array(&big);
    }
    o.u32(0);
    o.u8(0);
    o.u32(0);
    o.index_array(&big);
    o.u32(0);
    o.u32(0); // properties
    tail(o);
}

/// The synthetic model with the knobs applied.
pub fn build(knobs: Knobs) -> Vec<u8> {
    build_parts(knobs).0
}

/// The model and the offset of its first LOD (the end of the header).
pub fn build_parts(knobs: Knobs) -> (Vec<u8>, usize) {
    if let Some(n) = knobs.repeat_lod0_to_eof {
        return build_repeated(knobs, n);
    }
    let mut o = Out::default();
    o.bytes(b"ODOL");
    o.u32(56);
    o.u32(knobs.lod_count.unwrap_or(2));
    o.f32(1.0);
    o.f32(1.0e13);
    model_info(&mut o, knobs);
    if knobs.unterminated_property_class {
        let n = o.pos();
        return (o.0, n);
    }
    if knobs.bare {
        o.u8(0); // has_animations
    } else {
        o.u8(1); // has_animations
        animations(&mut o, 2);
    }
    let starts = o.pos();
    o.zeros(16); // lod_start x2, lod_end x2
    o.u8(0);
    o.u8(0); // use_default
    let start0 = o.pos();
    lod0(&mut o, knobs);
    let end0 = o.pos();
    lod1(&mut o, knobs);
    let end1 = o.pos();
    let patch = |o: &mut Out, at: usize, v: usize| o.patch_u32(at, u32_of(v));
    patch(&mut o, starts, start0);
    patch(
        &mut o,
        starts + 4,
        knobs.lod1_start.map_or(end0, |v| v as usize),
    );
    patch(&mut o, starts + 8, end0);
    patch(&mut o, starts + 12, end1);
    (o.0, start0)
}

/// `n` copies of LOD 0, back to back, every one declaring `end` = the end of the file.
fn build_repeated(knobs: Knobs, n: usize) -> (Vec<u8>, usize) {
    let mut lod = Out::default();
    lod0(&mut lod, knobs);
    let mut o = Out::default();
    o.bytes(b"ODOL");
    o.u32(56);
    o.u32(u32_of(n));
    for _ in 0..n {
        o.f32(1.0);
    }
    model_info(&mut o, knobs);
    o.u8(1); // has_animations
    animations(&mut o, n);
    let starts = o.pos();
    o.zeros(8 * n);
    o.u8(0);
    o.u8(0); // use_default
    let first = o.pos();
    for _ in 0..n {
        o.bytes(&lod.0);
    }
    let eof = u32_of(o.pos());
    for i in 0..n {
        let start = u32_of(first + i * lod.0.len());
        o.patch_u32(starts + 4 * i, start);
        o.patch_u32(starts + 4 * n + 4 * i, eof);
    }
    (o.0, first)
}
