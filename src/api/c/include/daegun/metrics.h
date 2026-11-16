/* Metrics: the line, OS/2, post and typographic metrics, PDF flags, tracking and STAT. Each at a location
 * reads the face's tables as daegun_font_instance builds them there; at the default, and where a location
 * can't be built, the face's own tables answer. Rust's metrics module. */

#ifndef DAEGUN_METRICS_H
#define DAEGUN_METRICS_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* the line */

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_ascender(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, double *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_descender(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, double *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_cap_height(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, double *out);
/** In degrees, counterclockwise from vertical: negative for a face that leans right. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_italic_angle(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, double *out);

/** The box head states for every glyph, as stored: MVAR doesn't vary it. DAEGUN_ABSENT where it has none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_bounds(const daegun_font *font, daegun_bounds *out);

typedef struct {
    double ascent;
    double descent;
    double line_gap;
} daegun_line_metrics;

/** A line's metrics at a location, along DAEGUN_HORIZONTAL or DAEGUN_VERTICAL. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_line_metrics(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, int32_t orientation,
                         daegun_line_metrics *out);

/** Baseline to baseline: ascent - descent + line_gap. A call rather than a subtraction you write,
 * because `descent` is negative and the obvious `ascent + descent + line_gap` is off by twice it
 * while looking plausible on every font. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_line_metrics_line_height(const daegun_line_metrics *metrics, double *out);

/* PDF font descriptor flags: FixedPitch, Serif, Script and Italic as the font states them, and Symbolic
 * for a Windows symbol font or one that maps a character outside the Adobe standard Latin set, else
 * Nonsymbolic. Control codes don't count. */
#define DAEGUN_PDF_FIXED_PITCH  0x01u
#define DAEGUN_PDF_SERIF        0x02u
#define DAEGUN_PDF_SYMBOLIC     0x04u
#define DAEGUN_PDF_SCRIPT       0x08u
#define DAEGUN_PDF_NONSYMBOLIC  0x20u
#define DAEGUN_PDF_ITALIC       0x40u
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_pdf_flags(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, uint32_t *out);

/* OS/2 */

/** What OS/2 says, DAEGUN_ABSENT where the face states no OS/2 table. The has_ fields are Rust's Option: a
 * table too short to hold a field leaves it out, so a face may state no family class or no typographic
 * metrics at all. */
typedef struct {
    uint16_t version;
    uint16_t weight_class;   /**< usWeightClass, 1 to 1000 as CSS's font-weight: 400 regular, 700 bold */
    uint16_t width_class;    /**< usWidthClass, 1 ultra-condensed to 9 ultra-expanded, 5 normal */
    /** fsType, the embedding the font's license allows: 0 installable, bit 1 (0x2) restricted, bit 2 (0x4)
     * preview and print, bit 3 (0x8) editable, bit 8 (0x100) not subset, bit 9 (0x200) bitmaps only */
    uint16_t fs_type;
    uint16_t family_class;
    uint16_t selection;
    bool     has_weight_class;
    bool     has_width_class;
    bool     has_fs_type;
    bool     has_family_class;
    bool     has_selection;
    bool     has_win_metrics;
    bool     has_typo_metrics;
    double   win_ascent;
    double   win_descent;    /**< as OS/2 stores it, positive below the baseline */
    double   typo_ascender;
    double   typo_descender;
    double   typo_line_gap;
} daegun_os2;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_os2(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location, daegun_os2 *out);

/** The five selection predicates, from fsSelection, false where it is left out. Oblique and typo metrics
 * count in any version, though an OS/2 table before version 4 reserves both bits. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_os2_is_italic(const daegun_os2 *os2, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_os2_is_bold(const daegun_os2 *os2, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_os2_is_regular(const daegun_os2 *os2, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_os2_is_oblique(const daegun_os2 *os2, bool *out);
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_os2_uses_typo_metrics(const daegun_os2 *os2, bool *out);

/* typographic metrics */

/** The x-height from OS/2; underline from post; strikeout, subscript and superscript from OS/2. A has_ false
 * is a value the face leaves out, its fields then 0. */
typedef struct {
    bool   has_x_height;
    bool   has_underline;
    bool   has_strikeout;
    bool   has_subscript;
    bool   has_superscript;
    double x_height;
    double underline_position;
    double underline_thickness;
    double strikeout_position;
    double strikeout_thickness;
    double subscript_x_size;
    double subscript_y_size;
    double subscript_x_offset;
    double subscript_y_offset;
    double superscript_x_size;
    double superscript_y_size;
    double superscript_x_offset;
    double superscript_y_offset;
} daegun_typographic_metrics;

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_typographic_metrics(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                                daegun_typographic_metrics *out);

/* tracking */

/* The point size a run is tracked at when its options give none. */
#define DAEGUN_DEFAULT_POINT_SIZE 12

/** Tracking at a point size, along DAEGUN_HORIZONTAL or DAEGUN_VERTICAL: the trak table's normal track, 0
 * where the table isn't read. Shaping applies it only to a font that also has a STAT it reads. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_tracking(const daegun_font *font, double point_size, int32_t orientation, double *out);

/* STAT */

typedef struct daegun_stat daegun_stat;

/** Read as daegun_font_names reads: the STAT daegun_font_instance builds at a location, the face's own at the
 * default, which describes its whole design space. DAEGUN_ABSENT where the face has none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_stat(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                 daegun_stat *DAEGUN_NULLABLE *DAEGUN_NONNULL out);

/** One design axis STAT describes: its tag, as fvar's would be, where it falls when names are put together,
 * lowest first, and whether daegun_stat_axis_name will answer. */
typedef struct {
    daegun_tag tag;
    uint16_t   ordering;
    bool       has_name;
} daegun_stat_axis;

/** Its axes in STAT's order, BORROWED until the STAT handle is freed. */
DAEGUN_API const daegun_stat_axis *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_stat_axes(const daegun_stat *stat, size_t *out_count);
/** An axis's display name, BORROWED until the STAT handle is freed. DAEGUN_ABSENT when unnamed;
 * DAEGUN_INVALID_ARGUMENT past the last axis. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_stat_axis_name(const daegun_stat *stat, size_t index, daegun_str *out);
/** How many axis values STAT names; daegun_stat_value_at reads them. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_stat_value_count(const daegun_stat *stat, size_t *out);
/** The name to use where every value's name is elided, BORROWED as the handle's names all are.
 * DAEGUN_ABSENT where STAT names none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_stat_elided_fallback_name(const daegun_stat *stat, daegun_str *out);
DAEGUN_API void DAEGUN_CALL
daegun_stat_free(daegun_stat *DAEGUN_NULLABLE stat);

/* Which fields of daegun_stat_value mean anything. */
#define DAEGUN_STAT_SINGLE 0
#define DAEGUN_STAT_RANGE  1
#define DAEGUN_STAT_LINKED 2
#define DAEGUN_STAT_COMBO  3

/** One STAT axis value, with the four variants flattened into one shape, the same choice the paint
 * graph made, since C has no sum type and four parallel lists would have to be correlated by index. */
typedef struct {
    int32_t  kind;          /**< DAEGUN_STAT_* */
    uint16_t axis_index;    /**< meaningless for COMBO, which spans axes */
    bool     elidable;
    bool     has_name;      /**< whether daegun_stat_value_name will answer */
    bool     older_sibling; /**< describes other fonts of the family, not this one */
    double   value;         /**< SINGLE's value, RANGE's nominal, LINKED's value; 0 for COMBO */
    double   min;           /**< RANGE only */
    double   max;           /**< RANGE only */
    double   linked_value;  /**< LINKED only */
    uint32_t combo_start;   /**< COMBO only: where its pairs start in daegun_stat_combo_values */
    uint32_t combo_count;
} daegun_stat_value;

/** One (axis, value) pair of a COMBO. */
typedef struct {
    uint16_t axis_index;
    double   value;
} daegun_axis_value;

/** DAEGUN_INVALID_ARGUMENT past the last value. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_stat_value_at(const daegun_stat *stat, size_t index, daegun_stat_value *out);
/** BORROWED until the STAT handle is freed. DAEGUN_ABSENT when unnamed; DAEGUN_INVALID_ARGUMENT past the last
 * value. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_stat_value_name(const daegun_stat *stat, size_t index, daegun_str *out);
/** BORROWED. Index from a value's combo_start for combo_count entries. */
DAEGUN_API const daegun_axis_value *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_stat_combo_values(const daegun_stat *stat, size_t *out_count);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_line_metrics) == 24, "line metrics are three doubles");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_stat_axis) == 8, "a STAT axis is a tag, an ordering and a flag");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_os2, family_class) == 8, "OS/2 layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_typographic_metrics) == 112, "five flags, then thirteen doubles");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_typographic_metrics, x_height) == 8, "typographic metrics layout");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_os2) == 64, "OS/2 is its version, five codes and seven flags, then five doubles");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_os2, win_ascent) == 24, "OS/2 layout");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_stat_value) == 56, "its kind and flags, four doubles and two uint32");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_axis_value) == 16, "an axis value is a uint16 and a double");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_METRICS_H */
