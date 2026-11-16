# ODOL v56 layout (DayZ binarized models)

Clean-room format description for issue #84. It is a list of facts about the file layout; it contains no code.
It was written from public format notes and has **not yet been checked field by field**: the implementer
verifies it against the game's own models (all 31,914 `.p3d` files in the DayZ client, server and Experimental
PBOs are ODOL version 56) and corrects this document where the files disagree.

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
| lod_start | lod_count × u32 | absolute file offset of each LOD |
| lod_end | lod_count × u32 | absolute end offset. In small models it can point past the end of the file, so don't treat that as an error |
| use_default | lod_count × bool | |
| face defaults | … | layout is uncertain. Don't parse it; read each LOD by seeking to `lod_start` |

A `lod_start` at or past the end of the file is an error.

## Model info (v56)

In order:
`u32 index`, `f32 mem_lod_sphere`, `f32 geo_lod_sphere`, `u32 remarks`, `u32 and_hints`, `u32 or_hints`,
`vec3 aiming_center`, `u32 map_icon_color`, `u32 map_selected_color`, `f32 view_density`,
`vec3 bbox_min`, `vec3 bbox_max`, `vec3 bbox_visual_min`, `vec3 bbox_visual_max`,
`vec3 bounding_center`, `vec3 geometry_center`, `vec3 center_of_mass`, `3 × vec3 inverse_inertia`,
`bool auto_center`, `bool lock_auto_center`, `bool can_occlude`, `bool can_be_occluded`,
`f32 ht_min`, `f32 ht_max`, `f32 af_max`, `f32 mf_max`, `f32 m_fact`, `f32 t_body`,
`bool force_not_alpha`, `i32 sb_source` (0 visual, 1 shadow volume, 2 explicit, 3 none, 4 visual ex),
`bool prefer_shadow_volume`, `f32 shadow_offset`, `bool animated`,
**skeleton** (below), `u8 map_type`,
**mass array**: `u32 count` + compressed array of `f32` (element size 4),
`f32 mass`, `f32 inv_mass`, `f32 alt_mass`, `f32 inv_alt_mass`,
`u8 geometry_simple`, `u8 geometry_phys`,
`u8 memory`, `u8 geometry`, `u8 geometry_fire`, `u8 geometry_view`, `u8 geometry_view_pilot`,
`u8 geometry_view_gunner`, `i8 (unknown)`, `u8 geometry_view_cargo`, `u8 land_contact`, `u8 roadway`, `u8 paths`,
`u8 hitpoints` (these are LOD indices of the special LODs, 0xFF / -1 = none),
`u32 min_shadow`, `bool can_blend`, `cstr property_class`, `cstr property_damage`, `bool property_frequent`,
`u32 n` + `n × cstr` (usually 0).

(Three per-LOD `i32` arrays of preferred shadow LODs only exist from v57, so they're not in v56.)

**Skeleton**: `cstr name`. If the name isn't empty: `bool is_discrete`, `u32 bone_count`,
`bone_count × (cstr bone, cstr parent)`, `cstr pivots_name` (obsolete). If the name is empty, none of these follow.

Useful for `inspect`: `property_class`, `property_damage`, skeleton name and bone count, `mass`, the bounding box.

## Animations (only if `has_animations`)

`u32 class_count`, then `class_count` animation classes:

`u32 type` (0 rotation, 1 rotationX, 2 rotationY, 3 rotationZ, 4 translation, 5 translationX, 6 translationY,
7 translationZ, 8 direct, 9 hide), `cstr name`, `cstr source`, `f32 min_phase`, `f32 max_phase`,
`f32 min_value`, `f32 max_value`, `f32 anim_period`, `f32 init_phase`, `u32 source_address`
(0 clamp, 1 loop, 2 mirror). Then, depending on the type:
- 0–3: `f32 angle0`, `f32 angle1`
- 4–7: `f32 offset0`, `f32 offset1`
- 8: `vec3 axis_pos`, `vec3 axis_dir`, `f32 angle`, `f32 axis_offset`
- 9: `f32 hide_value`, `f32 (unknown)`

Then `i32 res_count` (equal to `lod_count`). Then `res_count` blocks of bones-to-animations:
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
| faces_byte_size | u32 (total bytes of the face block that follows) |
| (zero) | u16 |
| faces | `face_count ×` (`u8 n` (3 or 4), `n × u16` vertex index) |
| section_count | u32, then sections (below) |
| selection_count | u32, then named selections (below) |
| property_count | u32, then `property_count × (cstr name, cstr value)` |
| frame_count | u32, then each: `f32 time`, `u32 n`, `n × vec3` |
| icon_color, selected_color, special | u32 ×3 |
| vertex_bone_ref_is_simple | bool |
| size_of_rest | u32 |
| … | clip flags (condensed array of u32), UV sets, positions, normals, … (not needed; stop here) |

The reader stops after `property_count` (or after the frames). Every field read has to stay before `lod_end[i]`
when `lod_end[i]` is inside the file.

**Material**: `cstr name`, `u32 mat_version`, `color emissive`, `color ambient`, `color diffuse`,
`color forced_diffuse`, `color specular`, `color specular2`, `f32 specular_power`, `i32 pixel_shader`,
`i32 vertex_shader`, `i32 main_light`, `i32 fog_mode`; if `mat_version == 3`: `bool`; if `mat_version ≥ 6`:
`cstr surface`; if `mat_version ≥ 4`: `u32 render_flag_count`, `u32 render_flags`; if `mat_version > 6`:
`u32 stage_count`; if `mat_version > 8`: `u32 transform_count`. Then `stage_count` stage textures, then
`transform_count` × (`u32 uv_source`, `matrix`). If `mat_version ≥ 10`, one more stage texture follows (a
dummy). **Stage texture**: if `mat_version ≥ 5`: `u32 render_flags`; `cstr texture`; if `mat_version ≥ 8`:
`u32 stage_id`; if `mat_version ≥ 11`: `bool use_world_env`. Record the `mat_version` values seen in the game
files in this document.

**Section**: `u32 face_start`, `u32 face_end` (byte offsets into the face block), `u32 min_bone`,
`u32 bone_count`, `u32 point_user_value`, `i16 texture_index`, `u32 face_flags`, `i32 material_index`; if
`material_index == -1`: `cstr material`; `u32 stage_count`, `stage_count × f32`.

**Named selection**: `cstr name`, index array `faces`, `u32 (always 0)`, `bool is_sectional`, then
`u32 count` + compressed array of `i32` (section indices, element size 4), index array `vertices`, then `u32 count`
+ compressed array of `u8` (vertex weights, element size 1).

## Limits (hostile input)

Every count gets checked against the bytes left before it's used to allocate. Strings are capped at a sane
length (for example 64 KiB). The number of LODs, proxies, textures, materials, sections, selections and
properties is capped (for example 4096 each), and decompressed arrays are capped (for example 64 MiB). An LOD
that fails to parse is reported as an error naming the LOD index and the file offset.
