// Every handle the C API gives a C++ caller, held in a std::unique_ptr whose deleter is its _free. The gate runs it
// under ASan with no spare handles, and leak checking on Linux, so a handle freed twice, or never, fails.
#include "daegun.h"

#include <cstdio>
#include <cstring>
#include <fstream>
#include <iterator>
#include <memory>
#include <string_view>
#include <utility>
#include <vector>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::fprintf(stderr, "  FAIL %s (last error: %s)\n", what, daegun_last_error().data);
        ++failures;
    }
}

template <class T, void (*Free)(T *)>
struct Deleter {
    void operator()(T *handle) const noexcept { Free(handle); }
};

template <class T, void (*Free)(T *)>
using Owned = std::unique_ptr<T, Deleter<T, Free>>;
static_assert(sizeof(Owned<daegun_font, daegun_font_free>) == sizeof(daegun_font *), "a stateless deleter");

// The out-parameter a call writes its handle to, taken by the owner when the full expression ends, as C++23's
// std::out_ptr does: so the owner is empty until that statement is over, and the handle is used in the next.
template <class P>
class OutPtr {
public:
    explicit OutPtr(P &owner) : owner_(owner) {}
    OutPtr(const OutPtr &) = delete;
    OutPtr &operator=(const OutPtr &) = delete;
    ~OutPtr() { owner_.reset(raw_); }
    operator typename P::pointer *() noexcept { return &raw_; }

private:
    P &owner_;
    typename P::pointer raw_ = nullptr;
};

template <class P>
OutPtr<P> out(P &owner)
{
    return OutPtr<P>(owner);
}

using Font = Owned<daegun_font, daegun_font_free>;
using Location = Owned<daegun_location, daegun_location_free>;
using Text = Owned<daegun_text, daegun_text_free>;
using Blob = Owned<daegun_blob, daegun_blob_free>;

// A list of fixed-size items as BUFFERS gives it: its count asked for with no buffer, then a vector filled.
template <typename T, typename Call> std::vector<T> filled(Call call)
{
    size_t n = 0;
    check(call(static_cast<T *>(nullptr), size_t(0), &n) == DAEGUN_OK, "a list's count");
    std::vector<T> items(n);
    check(call(items.data(), items.size(), &n) == DAEGUN_OK && n == items.size(), "a list filled");
    return items;
}

// A font buffer frees with the length it was made for, so its deleter carries that.
struct BufferFree {
    size_t len;
    void operator()(uint8_t *data) const noexcept { daegun_font_buffer_free(data, len); }
};

constexpr const char *INTER = "assets/test-fonts/inter/InterVariable.ttf";
constexpr daegun_tag WGHT = DAEGUN_TAG('w', 'g', 'h', 't');

std::vector<uint8_t> slurp(const char *path)
{
    std::ifstream in(path, std::ios::binary);
    return std::vector<uint8_t>(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
}

Font open(const char *path)
{
    std::vector<uint8_t> bytes = slurp(path);
    Font font;
    check(daegun_font_open(bytes.data(), bytes.size(), out(font)) == DAEGUN_OK, path);
    return font;
}

std::string_view view(daegun_str s)
{
    return std::string_view(s.data, s.len);
}

// The font itself, what it says, and the handles a caller copies tables and subsets into.
void a_font_and_what_it_hands_out()
{
    std::vector<uint8_t> bytes = slurp(INTER);
    std::unique_ptr<uint8_t, BufferFree> buffer(daegun_font_buffer_new(bytes.size()), BufferFree{ bytes.size() });
    check(buffer != nullptr, "a font buffer");
    if (!buffer) return;
    std::memcpy(buffer.get(), bytes.data(), bytes.size());
    std::unique_ptr<uint8_t, BufferFree> unused(daegun_font_buffer_new(64), BufferFree{ 64 });

    // open_owned takes the buffer pass or fail, so the owner lets go of it first.
    Font font;
    check(daegun_font_open_owned(buffer.release(), bytes.size(), 0, out(font)) == DAEGUN_OK, "open_owned");
    // A handle moves like any unique_ptr, and what it lent stays valid with it.
    std::vector<Font> fonts;
    fonts.push_back(std::move(font));
    const daegun_font *inter = fonts.back().get();

    daegun_coord bold_at{ WGHT, 700.0 };
    Location bold;
    check(daegun_font_location(inter, &bold_at, 1, out(bold)) == DAEGUN_OK, "a location");

    auto axis = filled<daegun_axis>([&](daegun_axis *o, size_t len, size_t *count) {
        return daegun_font_axes(inter, o, len, count);
    });
    check(axis.size() == 2 && axis[1].tag == WGHT, "Inter's axes are opsz and wght");
    size_t n = 0;

    Owned<daegun_named_instances, daegun_named_instances_free> named;
    daegun_str name{ nullptr, 0 };
    check(daegun_font_named_instances(inter, out(named)) == DAEGUN_OK, "named instances");
    check(daegun_named_instances_name(named.get(), 0, &name) == DAEGUN_OK && view(name) == "Thin", "instance 0");

    Text family;
    daegun_str s{ nullptr, 0 };
    check(daegun_font_family_name(inter, bold.get(), out(family)) == DAEGUN_OK, "family name");
    check(daegun_text_str(family.get(), &s) == DAEGUN_OK && view(s) == "Inter Variable", "the family's text");

    Owned<daegun_names, daegun_names_free> names;
    uint16_t id = 0;
    check(daegun_font_names(inter, nullptr, out(names)) == DAEGUN_OK, "names");
    check(daegun_names_count(names.get(), &n) == DAEGUN_OK && n > 10
              && daegun_names_at(names.get(), 1, &id, &s) == DAEGUN_OK && id == 1 && view(s) == "Inter Variable",
          "name ID 1 is the family");

    auto tags = filled<daegun_tag>([&](daegun_tag *o, size_t len, size_t *count) {
        return daegun_font_table_tags(inter, o, len, count);
    });
    check(tags.size() > 10, "more than ten tables");

    Owned<daegun_table_map, daegun_table_map_free> tables;
    Blob built;
    check(daegun_font_instance_tables(inter, bold.get(), out(tables)) == DAEGUN_OK, "instance tables");
    check(daegun_table_map_build(tables.get(), out(built)) == DAEGUN_OK, "building the instance");
    const uint8_t *data = daegun_blob_data(built.get(), &n);
    Font pinned;
    bool variable = true;
    check(data && daegun_font_open(data, n, out(pinned)) == DAEGUN_OK, "the instance opens");
    check(daegun_font_is_variable(pinned.get(), &variable) == DAEGUN_OK && !variable, "the instance is static");

    Owned<daegun_stat, daegun_stat_free> stat;
    check(daegun_font_stat(inter, nullptr, out(stat)) == DAEGUN_OK, "STAT");
    check(daegun_stat_value_count(stat.get(), &n) == DAEGUN_OK && n == 12, "Inter's STAT names 12 values");

    Owned<daegun_subset, daegun_subset_free> subset;
    uint32_t a = 0, mapped = 0;
    check(daegun_font_subset_text(inter, "AB", 2, nullptr, out(subset)) == DAEGUN_OK, "a subset");
    check(daegun_font_glyph_id(inter, 'A', &a) == DAEGUN_OK
              && daegun_subset_renumbered(subset.get(), a, &mapped) == DAEGUN_OK && mapped != 0,
          "the subset keeps A");

    std::vector<double> c(4);
    std::vector<uint8_t> present(c.size());
    Font fixture = open("assets/test-fonts/test-fixtures/carets.ttf");
    check(daegun_font_ligature_carets(fixture.get(), 6, nullptr, DAEGUN_HORIZONTAL, c.data(), present.data(), c.size(),
                                      &n)
              == DAEGUN_OK,
          "ligature carets");
    check(n == 3 && c[1] == 500.0 && present[1] == 1, "f_f_l's carets");
}

// What shaping, bidi and layout lend: each run borrowed from its owner, never freed apart from it.
void text_and_the_runs_it_shapes_to()
{
    Font font = open(INTER);
    Owned<daegun_shaped_run, daegun_shaped_run_free> run;
    size_t n = 0;
    check(daegun_font_shape(font.get(), "Hi", 2, nullptr, nullptr, out(run)) == DAEGUN_OK, "shaping");
    const uint32_t *glyphs = daegun_shaped_run_glyphs(run.get(), &n);
    check(glyphs && n == 2 && glyphs[0] == 161 && glyphs[1] == 689, "Hi shapes to H and i");

    static const char mixed[] = "abc \xD7\x90\xD7\x91\xD7\x92";
    Owned<daegun_bidi_runs, daegun_bidi_runs_free> runs;
    const daegun_shaped_run *borrowed = nullptr;
    check(daegun_font_shape_bidi(font.get(), mixed, sizeof mixed - 1, nullptr, DAEGUN_BASE_AUTO, nullptr, out(runs))
              == DAEGUN_OK,
          "bidi shaping");
    check(daegun_bidi_runs_count(runs.get(), &n) == DAEGUN_OK && n == 2
              && daegun_bidi_runs_at(runs.get(), 1, &borrowed, nullptr, nullptr, nullptr) == DAEGUN_OK
              && daegun_shaped_run_glyphs(borrowed, &n) && n == 3,
          "two bidi runs, the Hebrew three glyphs");

    Owned<daegun_bidi_paragraph, daegun_bidi_paragraph_free> paragraph;
    uint8_t level = 9;
    check(daegun_bidi_paragraph_new(mixed, sizeof mixed - 1, DAEGUN_BASE_RTL, out(paragraph)) == DAEGUN_OK, "bidi");
    check(daegun_bidi_paragraph_base_level(paragraph.get(), &level) == DAEGUN_OK && level == 1, "a level of 1");

    daegun_layout_options opts;
    check(daegun_layout_options_default(&opts, sizeof opts) == DAEGUN_OK, "layout defaults");
    opts.max_inline_size = 12288.0;
    Owned<daegun_layout, daegun_layout_free> layout;
    daegun_layout_info info;
    daegun_layout_run first;
    static const char fox[] = "the quick brown fox jumps over the lazy dog";
    check(daegun_font_layout(font.get(), fox, sizeof fox - 1, nullptr, &opts, out(layout)) == DAEGUN_OK, "layout");
    check(daegun_layout_info_of(layout.get(), &info) == DAEGUN_OK && info.line_count > 1
              && daegun_layout_run_at(layout.get(), 0, 0, &first) == DAEGUN_OK
              && daegun_shaped_run_glyphs(first.run, &n) && n > 0,
          "the fox wraps, its runs lent by the layout");

    // One JSTF priority level whose extension enables GSUB lookup 0, added to Inter.
    static const uint8_t jstf[46] = {
        0, 1, 0, 0, 0, 1, 'l', 'a', 't', 'n', 0, 12, 0, 0, 0, 6, 0, 0, 0, 1, 0, 4, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0,
    };
    Owned<daegun_table_map, daegun_table_map_free> tables;
    Blob built;
    Font justified;
    check(daegun_font_instance_tables(font.get(), nullptr, out(tables)) == DAEGUN_OK, "Inter's tables");
    check(daegun_table_map_set(tables.get(), DAEGUN_TAG('J', 'S', 'T', 'F'), jstf, sizeof jstf) == DAEGUN_OK,
          "adding JSTF");
    check(daegun_table_map_build(tables.get(), out(built)) == DAEGUN_OK, "building Inter with JSTF");
    const uint8_t *data = daegun_blob_data(built.get(), &n);
    check(data && daegun_font_open(data, n, out(justified)) == DAEGUN_OK, "Inter with JSTF opens");
    Owned<daegun_jstf_priorities, daegun_jstf_priorities_free> levels;
    check(daegun_font_justification_priorities(justified.get(), DAEGUN_TAG('l', 'a', 't', 'n'), 0, out(levels))
              == DAEGUN_OK,
          "JSTF priorities");
    check(daegun_jstf_priorities_count(levels.get(), &n) == DAEGUN_OK && n == 1, "one priority level");
}

struct Tally {
    int moves = 0, segments = 0, closes = 0;
};

// Outlines, as paths, contours, quads and hinted points, and a pen of C++ lambdas, which convert to the C
// callbacks it takes as long as they capture nothing; the state goes through `user`.
void outlines_and_what_holds_them()
{
    Font font = open(INTER);
    uint32_t a = 0;
    check(daegun_font_glyph_id(font.get(), 'A', &a) == DAEGUN_OK, "Inter's A");

    Tally tally;
    daegun_pen pen{
        [](void *u, float, float) noexcept { static_cast<Tally *>(u)->moves++; },
        [](void *u, float, float) noexcept { static_cast<Tally *>(u)->segments++; },
        [](void *u, float, float, float, float) noexcept { static_cast<Tally *>(u)->segments++; },
        [](void *u, float, float, float, float, float, float) noexcept { static_cast<Tally *>(u)->segments++; },
        [](void *u) noexcept { static_cast<Tally *>(u)->closes++; },
        &tally,
    };
    check(daegun_font_outline(font.get(), a, nullptr, &pen) == DAEGUN_OK && tally.moves == 2
              && tally.closes == tally.moves && tally.segments > 0,
          "A draws two contours into a pen of lambdas");

    Owned<daegun_path, daegun_path_free> path(daegun_path_new());
    daegun_pen into;
    check(path && daegun_path_as_pen(path.get(), &into) == DAEGUN_OK
              && daegun_font_outline(font.get(), a, nullptr, &into) == DAEGUN_OK,
          "A drawn into a path");
    Owned<daegun_contours, daegun_contours_free> contours;
    size_t n = 0;
    check(daegun_path_flatten(path.get(), daegun_max_area_for(16.0f, 2048.0f), out(contours)) == DAEGUN_OK,
          "flattening A");
    check(daegun_contours_count(contours.get(), &n) == DAEGUN_OK && n == 2, "A flattens to two contours");

    Owned<daegun_quads, daegun_quads_free> quads;
    check(daegun_font_quads(font.get(), a, nullptr, out(quads)) == DAEGUN_OK, "A's quads");
    check(daegun_quads_data(quads.get(), &n) && n > 0, "A has quads");

    Font hinted = open("assets/test-fonts/test-fixtures/hinted.ttf");
    uint32_t gid = 0;
    Owned<daegun_hinted_outline, daegun_hinted_outline_free> points;
    check(daegun_font_glyph_id(hinted.get(), 'A', &gid) == DAEGUN_OK
              && daegun_font_hinted_outline(hinted.get(), gid, nullptr, 16.0f, DAEGUN_HINT_FONT, out(points))
                     == DAEGUN_OK,
          "a hinted A");

    Font stix = open("assets/test-fonts/stix-two-math/STIX2Math.otf");
    Owned<daegun_cff_hints, daegun_cff_hints_free> hints;
    check(daegun_font_glyph_id(stix.get(), 'H', &gid) == DAEGUN_OK
              && daegun_font_cff_hints(stix.get(), gid, nullptr, out(hints)) == DAEGUN_OK,
          "STIX's H has CFF hints");

    uint32_t paren = 0;
    Owned<daegun_math_construction, daegun_math_construction_free> construction;
    check(daegun_font_glyph_id(stix.get(), '(', &paren) == DAEGUN_OK
              && daegun_font_math_glyph_variants(stix.get(), paren, nullptr, DAEGUN_VERTICAL, out(construction))
                     == DAEGUN_OK,
          "( stretches vertically");
    check(daegun_math_construction_variants(construction.get(), &n) && n == 13, "( has 13 vertical sizes");
}

// Color: COLR layers and palettes, paint graphs, scenes and their ramps, and a bitmap.
void color_and_what_draws_it()
{
    Font bungee = open("assets/test-fonts/bungee-tint/BungeeTint-Regular.ttf");
    uint32_t a = 0;
    size_t n = 0;
    check(daegun_font_glyph_id(bungee.get(), 'A', &a) == DAEGUN_OK, "Bungee has an A");
    auto layers = filled<daegun_colr_layer>([&](daegun_colr_layer *o, size_t len, size_t *count) {
        return daegun_font_colr_layers(bungee.get(), a, 0, o, len, count);
    });
    check(!layers.empty(), "Bungee's A has layers");
    auto palettes = filled<daegun_palette>([&](daegun_palette *o, size_t len, size_t *count) {
        return daegun_font_palettes(bungee.get(), o, len, count);
    });
    check(palettes.size() > 1, "Bungee has more than one palette");

    Font colr = open("assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    uint32_t glyphs = 0;
    check(daegun_font_glyph_count(colr.get(), &glyphs) == DAEGUN_OK, "the COLR font's glyphs");
    Owned<daegun_paint_graph, daegun_paint_graph_free> graph;
    Owned<daegun_scene, daegun_scene_free> scene;
    Owned<daegun_ramp, daegun_ramp_free> ramp;
    static const double identity[6] = { 1, 0, 0, 1, 0, 0 };
    for (uint32_t gid = 1; gid < glyphs && !ramp; gid++) {
        if (daegun_font_color_scene(colr.get(), gid, nullptr, nullptr, out(scene)) != DAEGUN_OK) continue;
        if (daegun_scene_gradients(scene.get(), &n) && n > 0) {
            check(daegun_scene_ramp(scene.get(), 0, identity, DAEGUN_INTERPOLATE_SRGB, out(ramp)) == DAEGUN_OK,
                  "a gradient's ramp");
            check(daegun_font_paint_graph(colr.get(), gid, nullptr, 0, out(graph)) == DAEGUN_OK,
                  "the same glyph's paint graph");
            check(daegun_paint_graph_nodes(graph.get(), &n) && n > 0, "the graph has paints");
        }
    }
    check(ramp && graph, "no scene in the COLR font holds a gradient");

    Font emoji = open("assets/test-fonts/noto-color-emoji/NotoColorEmoji.ttf");
    uint32_t smile = 0;
    Owned<daegun_bitmap, daegun_bitmap_free> bitmap;
    daegun_bitmap_info info;
    check(daegun_font_glyph_id(emoji.get(), 0x1F600, &smile) == DAEGUN_OK
              && daegun_font_bitmap(emoji.get(), smile, 64, out(bitmap)) == DAEGUN_OK,
          "a bitmap for U+1F600");
    check(daegun_bitmap_info_of(bitmap.get(), &info) == DAEGUN_OK && info.format == DAEGUN_BITMAP_PNG,
          "U+1F600 is a PNG");
}

// BASE, the format walkers over hand-built and real tables, and the raster helpers.
void tables_walkers_and_raster()
{
    Font han = open("assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    Owned<daegun_base_script, daegun_base_script_free> base;
    size_t n = 0;
    check(daegun_font_base_script(han.get(), DAEGUN_TAG('h', 'a', 'n', 'i'), nullptr, DAEGUN_HORIZONTAL, out(base))
              == DAEGUN_OK,
          "hani's baselines");
    check(daegun_base_script_baselines(base.get(), &n) && n == 4, "hani has four baselines");

    Font inter = open(INTER);
    daegun_bytes table{ nullptr, 0 };
    int16_t format = 0;
    uint32_t glyphs = 0;
    check(daegun_font_table(inter.get(), DAEGUN_TAG('h', 'e', 'a', 'd'), &table) == DAEGUN_OK
              && daegun_read_i16_be(table.data, table.len, 50, &format) == DAEGUN_OK
              && daegun_font_glyph_count(inter.get(), &glyphs) == DAEGUN_OK
              && daegun_font_table(inter.get(), DAEGUN_TAG('l', 'o', 'c', 'a'), &table) == DAEGUN_OK,
          "head and loca");
    auto offsets = filled<size_t>([&](size_t *o, size_t len, size_t *count) {
        return daegun_parse_loca(table.data, table.len, format, glyphs, o, len, count);
    });
    check(offsets.size() == glyphs + size_t(1), "an offset for each glyph and one");

    uint32_t store_at = 0, map_at = 0;
    Owned<daegun_ivs, daegun_ivs_free> ivs;
    Owned<daegun_delta_set_index_map, daegun_delta_set_index_map_free> map;
    check(daegun_font_table(inter.get(), DAEGUN_TAG('H', 'V', 'A', 'R'), &table) == DAEGUN_OK
              && daegun_read_u32_be(table.data, table.len, 4, &store_at) == DAEGUN_OK
              && daegun_read_u32_be(table.data, table.len, 8, &map_at) == DAEGUN_OK
              && daegun_ivs_parse(table.data, table.len, store_at, out(ivs)) == DAEGUN_OK
              && daegun_delta_set_index_map_parse(table.data, table.len, map_at, out(map)) == DAEGUN_OK,
          "HVAR's store and map");
    check(daegun_delta_set_index_map_count(map.get(), &n) == DAEGUN_OK && n == glyphs, "the map covers each glyph");

    static const uint8_t single[20] = { 0, 6, 0, 4, 0, 2, 0, 8, 0, 1, 0, 0, 0, 3, 0, 30, 0, 7, 0, 70 };
    Owned<daegun_aat_lookup, daegun_aat_lookup_free> lookup;
    check(daegun_aat_lookup_parse(single, sizeof single, 10, out(lookup)) == DAEGUN_OK, "an AAT lookup");
    auto entries = filled<daegun_glyph_value>([&](daegun_glyph_value *o, size_t len, size_t *count) {
        return daegun_aat_lookup_entries(lookup.get(), o, len, count);
    });
    check(entries.size() == 2, "two mappings");

    static const uint8_t machine[56] = {
        0, 0, 0, 5, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0, 44, 0, 0, 0, 4, 0, 4, 0, 3, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x80, 0, 0x12, 0x34,
    };
    Owned<daegun_aat_state_table, daegun_aat_state_table_free> states;
    check(daegun_aat_state_table_parse(machine, sizeof machine, 1, 3, out(states)) == DAEGUN_OK, "a state table");

    static const uint8_t anchors[36] = {
        0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 20, 0, 0, 0, 12, 0, 0, 0, 12,
        0, 0, 0, 2, 0, 10, 0, 20, 0xFF, 0xE2, 0, 40, 0, 0, 0, 0,
    };
    Owned<daegun_ankr, daegun_ankr_free> ankr;
    check(daegun_ankr_parse(anchors, sizeof anchors, 3, out(ankr)) == DAEGUN_OK, "an ankr");

    static const uint8_t variations[46] = {
        0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 30, 0, 1, 0, 0, 0, 6, 0, 1,
        0, 0, 0x20, 0, 0x40, 0, 0, 1, 0, 0, 0, 1, 0, 3, 0, 0, 0, 12, 0, 0, 0, 0,
    };
    Owned<daegun_feature_variations, daegun_feature_variations_free> fv;
    check(daegun_feature_variations_at(variations, sizeof variations, 0, out(fv)) == DAEGUN_OK, "FeatureVariations");

    Owned<daegun_subpixel_layout, daegun_subpixel_layout_free> rgb;
    uint8_t x = 0, y = 0;
    check(daegun_subpixel_layout_horizontal(DAEGUN_STRIPES_RGB, out(rgb)) == DAEGUN_OK, "an RGB layout");
    check(daegun_subpixel_layout_oversample(rgb.get(), &x, &y) == DAEGUN_OK && x == 3 && y == 1, "3x1 samples");

    Owned<daegun_shelf_packer, daegun_shelf_packer_free> packer(daegun_shelf_packer_new(64, 64));
    daegun_rect placed{ 0, 0, 0, 0 };
    check(packer && daegun_shelf_packer_insert(packer.get(), 16, 16, &placed) == DAEGUN_OK && placed.w == 16,
          "a packed rectangle");

    Owned<daegun_rasterizer, daegun_rasterizer_free> rasterizer(daegun_rasterizer_new());
    Owned<daegun_path, daegun_path_free> triangle(daegun_path_new());
    check(daegun_path_move_to(triangle.get(), 0.0f, 0.0f) == DAEGUN_OK
              && daegun_path_line_to(triangle.get(), 3.0f, 0.0f) == DAEGUN_OK
              && daegun_path_line_to(triangle.get(), 3.0f, 2.0f) == DAEGUN_OK
              && daegun_path_close(triangle.get()) == DAEGUN_OK,
          "a triangle");
    daegun_image image{};
    check(daegun_rasterizer_path(rasterizer.get(), triangle.get(), DAEGUN_FILL_NONZERO, nullptr, &image) == DAEGUN_OK
              && image.width == 3 && image.height == 2 && image.channels == 1 && image.pixels_len == 6,
          "a triangle drawn into a borrowed image");
}

} // namespace

int main()
{
    std::printf("daegun C++ handles\n");
    a_font_and_what_it_hands_out();
    text_and_the_runs_it_shapes_to();
    outlines_and_what_holds_them();
    color_and_what_draws_it();
    tables_walkers_and_raster();
    if (failures == 0) {
        std::printf("  ok\n");
        return 0;
    }
    std::printf("  %d failed\n", failures);
    return 1;
}
