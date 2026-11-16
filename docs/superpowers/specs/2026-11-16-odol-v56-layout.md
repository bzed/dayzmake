# ODOL v56 layout (DayZ binarized models)

Format description for issue #84. It is a list of facts about the file layout; it contains no code.

Provenance: the layout facts were extracted by reading the source of github.com/arma-tools/arma-file-formats-rs
(commit ea96a07, Nov 2026; the repository has no LICENSE file, and its `Cargo.toml` says `license = "MIT"`). Only
facts were taken: field order, sizes and version conditions. No code was copied. The facts were then checked
against, and corrected by, all 31,914 ODOL v56 models of DayZ: all 31,914 `.p3d` files in the DayZ client, server
and Experimental PBOs are ODOL version 56, and the reader in `crates/dayzmake-core/src/odol.rs` parses every one
of them (184,608 LODs). Where the files disagreed with the first draft, this document now says what the files
hold; those places are marked *(verified)* or *(corrected)*. The reader was written by implementers who had only
this spec and the game files.

Fields no tool of ours needs are skipped, and their meaning is a guess (marked *unknown*).

Notation: all integers are little endian. `u8/u16/u32/i16/i32/f32` as usual. `cstr` = bytes up to a NUL
(the NUL is consumed). `vec3` = 3 × `f32`. `color` = 4 × `f32` (r, g, b, a). `matrix` = 12 × `f32` (a 3×3
orientation followed by a position). `bool` = `u8`, non-zero = true.

## Compressed arrays

Many arrays are stored as a count followed by `count × element_size` bytes that may be compressed. For version 56:

- The bytes are LZO1X-compressed **if and only if** `count × element_size ≥ 1024`. Otherwise they are stored raw.
- The compressed length isn't stored. The LZO stream ends with its own end marker, and the next field starts
  right after it. The decoder therefore has to report how many input bytes it consumed. It must also stop once
  `count × element_size` output bytes are produced, and fail if the stream wants to produce more.
- A count of 0 has no bytes at all.
- *(verified)* In the game files 44,507 arrays are compressed (from 1,024 up to 183,216 bytes decompressed). Every
  array below 1,024 bytes is raw, and there is no array of 1,024 bytes or more that is raw.

"Index array" means a compressed array of `u16` (element size 2). Vertex and face indices are 16 bit in v56.

"Condensed array" (not needed for this task, only for skipping past it) = `u32 count`, `bool default_fill`; if
`default_fill`, one element follows. Otherwise a compressed array of `count` elements follows, with no second count.

## File header

| Field | Type | Notes |
|---|---|---|
| magic | 4 bytes | `ODOL` |
| version | u32 | 56 |
| lod_count | u32 | |
| resolutions | lod_count × f32 | LOD resolution values (same meaning as MLOD; reuse `p3d::lod_name`) |
| model_info | see below | |
| has_animations | bool | |
| animations | see below | only if `has_animations` |
| lod_start | lod_count × u32 | absolute file offset of each LOD. *(verified)* The LODs are normally stored in reverse order: the highest index first in the file, so the starts descend, and `lod_end[i]` is the start of the LOD that follows it in the file (the file length for the last one) |
| lod_end | lod_count × u32 | absolute end offset. In small models it can point past the end of the file, so don't treat that as an error |
| use_default | lod_count × bool | |
| face defaults | … | *(verified)* Between `use_default` and the lowest `lod_start`: 0 bytes in a one-LOD model, 25 bytes in a four-LOD sample, so it isn't a fixed size. Don't parse it; read each LOD by seeking to `lod_start` |

A `lod_start` at or past the end of the file is an error.

## Model info (v56)

In order:
`u32 index`, `f32 mem_lod_sphere`, `f32 geo_lod_sphere`, `u32 remarks`, `u32 and_hints`, `u32 or_hints`,
`vec3 aiming_center`, `u32 map_icon_color`, `u32 map_selected_color`, `f32 view_density`,
`vec3 bbox_min`, `vec3 bbox_max`, `vec3 bbox_visual_min`, `vec3 bbox_visual_max`,
`vec3 bounding_center`, `vec3 geometry_center`, `vec3 center_of_mass`, `3 × vec3 inverse_inertia`,
*(corrected)* **5** `bool` (`auto_center`, `lock_auto_center`, `can_occlude`, `can_be_occluded` and one unknown, in
the samples `1 0 0 1 1`), **8** `f32` (two unknown, then `ht_min`, `ht_max`, `af_max`, `mf_max`, `m_fact`, `t_body`;
the draft had 4 bools and 6 floats),
`bool force_not_alpha`, `i32 sb_source` (0 visual, 1 shadow volume, 2 explicit, 3 none, 4 visual ex),
`bool prefer_shadow_volume`, `f32 shadow_offset` (often `f32::MAX`), `bool animated`,
**skeleton** (below), `u8 map_type`,
**mass array**: `u32 count` + compressed array of `f32` (element size 4; empty in all but a few models),
`f32 mass`, `f32 inv_mass`, `f32 alt_mass`, `f32 inv_alt_mass`,
*(corrected)* **13** `u8` LOD indices of the special LODs (0xFF = none; the draft listed 14). From the models: the
first is the memory LOD, the second the geometry LOD, the fourth the fire geometry, the fifth the view geometry,
the eleventh the roadway; the rest are not identified and nobody needs them,
`u32 min_shadow`, `bool can_blend`, `cstr property_class`, `cstr property_damage`, `bool property_frequent`,
`u32 n` + `n × cstr` (always 0 in the game files),
*(corrected)* one more unknown `u8` (always 0; it sits after `n`, or between `property_frequent` and `n`, which
can't be told apart because `n` is always 0).

(Three per-LOD `i32` arrays of preferred shadow LODs only exist from v57, so they're not in v56.)

**Skeleton**: `cstr name`. If the name isn't empty: `bool is_discrete`, `u32 bone_count`,
`bone_count × (cstr bone, cstr parent)`, `cstr pivots_name` (obsolete). If the name is empty, none of these follow.

Useful for `inspect`: `property_class`, `property_damage`, skeleton name and bone count, `mass`, the bounding box.

## Animations (only if `has_animations`)

`u32 class_count`, then `class_count` animation classes:

`u32 type` (0 rotation, 1 rotationX, 2 rotationY, 3 rotationZ, 4 translation, 5 translationX, 6 translationY,
7 translationZ, 8 direct, 9 hide), `cstr name`, `cstr source`, *(corrected)* `f32 min_phase`, `f32 max_phase`,
`f32 min_value`, `f32 max_value`, `u32 source_address` (0 clamp, 1 loop, 2 mirror); the draft had two more floats
(`anim_period`, `init_phase`) here, which the files don't have. Then, depending on the type:
- 0–7: `f32 angle0`, `f32 angle1` (rotations) or `f32 offset0`, `f32 offset1` (translations)
- 8: `vec3 axis_pos`, `vec3 axis_dir`, `f32 angle`, `f32 axis_offset` (not seen in the game files; unverified)
- 9: *(corrected)* `f32 hide_value` only (the draft had a second, unknown float)

Then `u32 res_count`: `lod_count` normally. *(corrected)* A model can have `has_animations` set and no animation
classes at all (`class_count` 0): then `res_count` is 0 too and no blocks follow (165 game models).
Then `res_count` blocks of bones-to-animations:
`u32 bone_count`, `bone_count × (u32 n, n × u32 class_index)`.
Then `res_count` blocks of animations-to-bones. Each block has one entry per animation class (in class order):
`i32 bone_index`; if `bone_index != -1` and the class type isn't 8 (direct) or 9 (hide), `vec3 axis_pos` and
`vec3 axis_dir` follow.

Useful for `inspect`: the animation class names and sources.

## LOD (at `lod_start[i]`)

| Field | Type |
|---|---|
| proxy_count | u32 |
| proxies | each: `cstr model`, `matrix transform`, `i32 sequence_id`, `i32 named_selection_index`, `i32 bone_index`, `i32 section_index` |
| item_count | u32, then `item_count × u32` |
| bone_link_count | u32, then each: `u32 n`, `n × u32` |
| vertex_count | u32 |
| face_area | f32 |
| or_hints, and_hints | u32, u32 |
| bbox_min, bbox_max, bcenter | vec3 ×3 |
| bradius | f32 |
| texture_count | u32, then `texture_count × cstr` |
| material_count | u32, then materials (below) |
| edges | two index arrays (each: `u32 count` + compressed `u16` array) |
| face_count | u32 |
| faces_byte_size | u32. *(corrected)* It isn't the size of the bytes that follow: it is the sum over the faces of `2 × n + 2` (8 for a triangle, 10 for a quad), as if every face were a `u16` count plus `n` `u16` indices. It is also the unit of the sections' `face_start`/`face_end`. Verified on every face of every game model |
| (zero) | u16 |
| faces | `face_count ×` (`u8 n` (3 or 4), `n × u16` vertex index): `1 + 2 × n` bytes each |
| section_count | u32, then sections (below) |
| selection_count | u32, then named selections (below) |
| property_count | u32, then `property_count × (cstr name, cstr value)` |
| frame_count | u32, then each: `f32 time`, `u32 n`, `n × vec3` |
| icon_color, selected_color, special | u32 ×3 |
| vertex_bone_ref_is_simple | bool |
| size_of_rest | u32 |
| … | clip flags (condensed array of u32), UV sets, positions, normals, … (not needed; stop here) |

The reader stops after `property_count`. Every field read has to stay before `lod_end[i]` when `lod_end[i]` is
inside the file.

*(verified)* The fields after the properties are laid out as written: the frames (`u32 frame_count`, each `f32 time`,
`u32 n`, `n × vec3`), `3 × u32`, `bool`, `u32 size_of_rest`. The end of that `u32` minus 4 plus `size_of_rest` is
exactly `lod_end[i]` (the file length for the last LOD in the file) in all 184,608 LODs of the game files. This
was used as the check that the parse up to there is aligned; the reader itself doesn't need it.
In the game files a LOD has at most 2,048 selections, 447 sections, 60 textures, 21 materials, 1,057 proxies and
96,210 faces (65,530 vertices).

**Material**: `cstr name`, `u32 mat_version`. *(corrected)* **Every material in the game files has
`mat_version` 20** (28,349 of 28,349 files checked at their first material, and all 31,914 models parse with it);
the draft's rules for other versions (3 to 11) were from the notes and have never been seen, so the reader
refuses any other version with an error naming it. For version 20 the fields are:
200 bytes of colors and lighting values (more than the draft's six colors; not decoded), `f32 specular_power`,
`i32 pixel_shader`, `i32 vertex_shader`, `i32 main_light`, `i32 fog_mode`, `cstr surface`, `u32 render_flag_count`,
`u32 render_flags`, `u32 stage_count`, `u32 transform_count`. Then `stage_count` stage textures, then
`transform_count` × (`u32 uv_source`, `matrix`), then one more stage texture (a dummy, usually with an empty
path). **Stage texture**: `u32 render_flags`; `cstr texture`; `u32 stage_id`; `bool use_world_env`.

**Section**: `u32 face_start`, `u32 face_end` (byte offsets into the face block), `u32 min_bone`,
`u32 bone_count`, `u32 point_user_value`, `i16 texture_index`, `u32 face_flags`, `i32 material_index`; if
`material_index == -1`: `cstr material`; `u32 stage_count`, `stage_count × f32`.

**Named selection**: `cstr name`, index array `faces`, `u32 (always 0)`, `bool is_sectional`, then
`u32 count` + compressed array of `i32` (section indices, element size 4), index array `vertices`, then `u32 count`
+ compressed array of `u8` (vertex weights, element size 1). *(verified)*

## Limits (hostile input)

Every count gets checked against the bytes left before it's used to allocate. Strings are capped at 64 KiB. The
number of LODs, proxies, textures, materials, sections, selections, properties, bones and animation classes is
capped at 65,536 each (the game files stay far below: at most 2,048 selections per LOD), and decompressed arrays
are capped at 64 MiB, and all arrays of one file together at 256 MiB of decompressed data (LZO shrinks 255:1, so
without a total a file of a megabyte could ask for terabytes). The bytes of one LOD read into memory are capped at
256 MiB, and the ranges of the LODs must not overlap: a LOD that starts inside the data of the one before it is
refused, because LODs that share data would make a small file cost as much work as a big one. The LZO decoder is `lzokay-native` (it reports where the stream ended and stops at the
declared size); a stream that decompresses to anything but the declared size is an error. A LOD that fails to
parse is reported as an error naming the LOD index and the file offset.
