"""The skit Textual theme: btop-flavored, terminal-native.

Design contract:
- The canvas stays `ansi_default` (the user's terminal background — transparency and
  all — shows through, like btop with theme_background off), but everything skit draws
  ON that canvas uses a controlled btop-style palette: rounded panel borders in muted
  per-panel tints, near-white titles, a dark warm selection bar, and one warm accent
  (terracotta) for keys and focus.
- `ansi=True` disables Textual's ANSI→truecolor filter so default colors pass through
  natively — BUT the ansi color system also hard-defaults links, scrollbars and table
  headers to ANSI blue, which is unreadable noise on a dark terminal. Every one of
  those is overridden below (variables) or in CHROME_CSS (component styles).
"""

from __future__ import annotations

from textual.theme import Theme

# Anthropic terracotta — the one warm accent (close kin to btop's hi_fg red).
ACCENT = "#D97757"
ACCENT_DIM = "#B05C3F"

# Selection: btop highlights the current row with a dark tinted bar + bright text,
# never a solid accent bar (accent-on-black text stays readable, the bar stays calm).
SELECT_BG = "#5A2D1E"
SELECT_FG = "#EEEEEE"

# btop's signature: each panel wears its own muted border tint (cpu green, mem olive,
# net indigo, proc maroon). skit maps them to its own surfaces.
BOX_GREEN = "#3D7B46"  # Library list
BOX_OLIVE = "#8A882E"  # add flow
BOX_INDIGO = "#4B44B0"  # detail pane, settings, preferences
BOX_MAROON = "#923535"  # run form, destructive confirms
BOX_DIM = "#3A3A3A"  # idle input borders / dividers

# Exposed to screen CSS as $skit-box-*. Apps must merge these into get_css_variables():
# the first stylesheet parse happens BEFORE on_mount activates the skit theme, and an
# unresolved variable is a startup crash, not a fallback.
BOX_VARIABLES = {
    "skit-box-green": BOX_GREEN,
    "skit-box-olive": BOX_OLIVE,
    "skit-box-indigo": BOX_INDIGO,
    "skit-box-maroon": BOX_MAROON,
}

CLAUDE_THEME = Theme(
    name="skit-claude",
    primary=ACCENT,
    secondary=ACCENT_DIM,
    accent=ACCENT,
    warning="ansi_yellow",
    error="ansi_red",
    success="ansi_green",
    foreground="ansi_default",
    background="ansi_default",
    surface="ansi_default",
    panel="ansi_default",
    boost="ansi_default",
    dark=True,
    variables={
        # The full variable set an ansi theme must provide (Screen's builtin CSS reads
        # ansi-background/-foreground; the rest mirror the builtin ansi-dark theme).
        "ansi-background": "ansi_black",
        "ansi-foreground": "ansi_white",
        "border-blurred": BOX_DIM,
        # $border is what every :focus border falls back to (the ansi system would say
        # magenta); focus always speaks accent.
        "border": ACCENT,
        # Selection bar: dark terracotta + bright text (btop's selected_bg pattern) —
        # the old accent-background bar drowned the row it was meant to point at.
        "block-cursor-foreground": SELECT_FG,
        "block-cursor-background": SELECT_BG,
        "block-cursor-text-style": "bold",
        "block-cursor-blurred-foreground": "ansi_default",
        "block-cursor-blurred-background": "#33231C",
        "block-hover-background": "#33231C",
        "input-cursor-background": "ansi_black",
        "input-cursor-foreground": "ansi_bright_white",
        "input-cursor-text-style": "none",
        "input-selection-background": ACCENT_DIM,
        "input-selection-foreground": "ansi_black",
        "screen-selection-background": ACCENT_DIM,
        "screen-selection-foreground": "ansi_black",
        "footer-key-foreground": ACCENT,
        # Action links (footer chips, ▾insert, modal buttons): the ansi system paints
        # them underlined blue, which reads as a 1998 hyperlink. Textual applies link-color
        # uniformly to the whole @click span, overriding any inline color — so an inline
        # [$accent] on the chip's key can't win. Set link-color TO the accent: the key
        # (which keeps its `bold`) reads as bold-terracotta, the label as plain terracotta,
        # and the whole pill stays one clickable button. This also matches the ▾insert link,
        # which already intends accent. Hover brightens the whole pill.
        "link-color": ACCENT,
        "link-style": "none",
        "link-color-hover": "ansi_bright_white",
        "link-background-hover": "#4A2A1D",
        "link-style-hover": "none",
        # Panel tints, exposed to screen CSS as $skit-box-*.
        **BOX_VARIABLES,
        # Scrollbars: warm dark rail instead of ansi blue.
        "scrollbar": "#4A413C",
        "scrollbar-hover": ACCENT_DIM,
        "scrollbar-active": ACCENT,
        "scrollbar-background": "ansi_default",
        "scrollbar-background-hover": "ansi_default",
        "scrollbar-background-active": "ansi_default",
    },
    ansi=True,
)

# Component chrome no theme variable reaches, shared by every skit App (the workbench
# and the CLI's inline form). btop grammar: rounded borders, titles ON the border,
# bold near-white table headers on the bare canvas — never a colored header bar.
CHROME_CSS = """
Screen { background: ansi_default; }
/* The ansi color system pins the table header to ansi_bright_blue (unreadable and
   loud on a dark terminal); btop headers are just bold bright text on the canvas. */
DataTable:ansi > .datatable--header,
DataTable > .datatable--header { background: ansi_default; color: ansi_bright_white; text-style: bold; }
Input { border: round $border-blurred; }
Input:focus { border: round $accent; }
Select > SelectCurrent { border: round $border-blurred; }
Select:focus > SelectCurrent { border: round $accent; }
"""
