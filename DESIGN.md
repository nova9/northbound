---
name: TrueNorth Wallet
description: Civic Clarity for calm, explicit self-custody on desktop.
colors:
  ink-shell: "#172337"
  neutral-ground: "#f1f3f5"
  paper-surface: "#ffffff"
  muted-surface: "#edf1f4"
  service-blue: "#195da5"
  service-blue-hover: "#124b88"
  primary-on-color: "#ffffff"
  text-muted: "#526174"
  text-subtle: "#6b746f"
  secondary-fill: "#e8edf2"
  secondary-fill-hover: "#dce3e9"
  border: "#c6ced7"
  border-muted: "#e1e6ea"
  border-strong: "#9aa6b2"
  success: "#1f694a"
  error: "#a72f28"
  address-text: "#334055"
typography:
  display:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "clamp(2.7rem, 5vw, 4.25rem)"
    fontWeight: 620
    lineHeight: 1
    letterSpacing: "-0.035em"
  headline:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "1.75rem"
    fontWeight: 700
    lineHeight: 1.15
    letterSpacing: "-0.025em"
  title:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "1rem"
    fontWeight: 650
  body:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "0.8125rem"
    fontWeight: 650
  data:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  tag: "3px"
  control: "4px"
  action: "5px"
  large-control: "6px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  section: "24px"
  gutter: "28px"
  wide: "32px"
components:
  button-primary:
    backgroundColor: "{colors.service-blue}"
    textColor: "{colors.primary-on-color}"
    typography: "{typography.label}"
    rounded: "{rounded.large-control}"
    padding: "11px 18px"
    height: "44px"
  button-primary-hover:
    backgroundColor: "{colors.service-blue-hover}"
    textColor: "{colors.primary-on-color}"
  button-secondary:
    backgroundColor: "{colors.secondary-fill}"
    textColor: "{colors.ink-shell}"
    typography: "{typography.label}"
    rounded: "{rounded.large-control}"
    padding: "11px 18px"
    height: "44px"
  input:
    backgroundColor: "{colors.paper-surface}"
    textColor: "{colors.ink-shell}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "11px 12px"
    height: "46px"
  network-tag:
    backgroundColor: "#263950"
    textColor: "#dce5f0"
    rounded: "{rounded.tag}"
    padding: "4px 7px"
---

# Design System: TrueNorth Wallet

## Overview

**Creative North Star: "Civic Clarity"**

TrueNorth Wallet behaves like excellent public information: the important fact is immediate, the next action is obvious, and the interface never performs crypto theater. A deep ink shell anchors the window while a pale neutral working ground keeps amounts, addresses, and status language easy to scan.

The system is compact, flat, and operational. Hairline dividers create a timetable-like rhythm; color marks action and meaningful state rather than decoration. Language remains calm and explicit, especially when something fails: name the problem and give the recovery action without hype or alarm.

**Key Characteristics:**

- Compact desktop shell with horizontal navigation.
- Pale working ground under a deep ink header.
- Flat regions separated by hairlines instead of floating cards.
- Tabular amounts and monospace only for addresses and machine data.
- Small, square control corners and familiar native behavior.

## Colors

The palette is civic and restrained: deep ink establishes authority, service blue identifies action, and semantic colors appear only when a status warrants them.

### Primary

- **Service Blue:** The sole primary action color for buttons, links, input focus, and active emphasis.
- **Service Blue Hover:** A darker interaction state that preserves the same clear action hierarchy.

### Neutral

- **Deep Ink Shell:** The application header and primary text color; it gives the desktop window a stable frame.
- **Pale Neutral Ground:** The continuous application background; it should remain visible between functional regions.
- **Paper Surface:** Inputs and bounded data regions that need separation from the ground.
- **Muted Surface:** Quiet row hover and secondary information fill.
- **Muted Text / Subtle Text:** Supporting descriptions and placeholders, never the sole carrier of essential status.
- **Hairline Border / Muted Hairline / Strong Border:** Structural separators, row divisions, and input edges in increasing strength.

### Tertiary

- **Success Green:** Confirmed, current, ready, and security-positive states.
- **Signal Red:** Real errors and failed states only.

### Named Rules

**The Service Color Rule.** Service blue indicates something the user can do; it is not ambient decoration.

**The Explicit Status Rule.** Status always includes words. Green and red support meaning but never carry it alone.

## Typography

**Display Font:** System UI (`-apple-system`, `BlinkMacSystemFont`, `Segoe UI`, sans-serif)

**Body Font:** System UI (`-apple-system`, `BlinkMacSystemFont`, `Segoe UI`, sans-serif)

**Label/Mono Font:** `ui-monospace`, `SFMono-Regular`, `Menlo`, monospace for addresses and machine data only

**Character:** Native system typography keeps the interface familiar and untheatrical. Hierarchy comes from size, weight, spacing, and numeric alignment rather than a decorative brand face.

### Hierarchy

- **Display** (620, fluid 2.7–4.25rem, 1): Wallet balances only; always use tabular numerals.
- **Headline** (700, 1.75rem, 1.15): Screen titles and decisive empty-state headings.
- **Title** (650, 1rem): Section headings and compact functional grouping.
- **Body** (400, 1rem, 1.55): Instructions and explanatory copy, generally limited to 70 characters.
- **Label** (650, 0.8125rem): Navigation, controls, statuses, and data captions.
- **Data** (400, 0.8125rem, 1.5): Addresses and transaction identifiers; allow selection and safe wrapping.

### Named Rules

**The Numeric Alignment Rule.** Amounts, heights, fees, totals, and counts use tabular numerals so changing values do not disturb the scan line.

**The Data-Only Mono Rule.** Monospace is reserved for addresses and machine identifiers, never headings, buttons, or ordinary prose.

## Layout

The desktop shell is a full-height grid with a compact 62px header (86px when the macOS titlebar is integrated) and one scrollable content region. Content uses 28px horizontal gutters and a centered maximum width of 1080px; focused flows narrow to 620px. The primary navigation is horizontal inside the header, with wallet status and node controls aligned to the opposite edge.

Overview, Receive, Send, and History use two-column work areas where space allows. Columns are structural rather than card-based: 1px dividers and 20–32px gaps define the relationship. At 900px, paired regions stack and vertical dividers become horizontal. At 720px, the header condenses, action grids become single-column, content gutters reduce to 16px, and data rows reflow without hiding essential actions.

**The Continuous Ground Rule.** Do not turn the wallet into a sidebar dashboard or a stack of detached cards. Preserve one continuous ground and divide work with alignment, spacing, and hairlines.

## Elevation & Depth

The system is flat. It uses no ambient drop shadows and no glass effects. Depth comes from the dark shell, small tonal changes, bordered fields, and hairline section boundaries. Focus is explicit through a 3px translucent blue outline with a 2px offset.

**The Flat-by-Default Rule.** A surface may change tone on hover, but it does not lift, glow, or acquire a decorative shadow.

## Shapes

Controls are square with just enough softening to read as interactive: 3px for tags, 4px for fields and compact controls, 5px for task actions, and 6px for standard buttons or bounded utility regions. Information sections themselves remain unrounded and are defined by straight hairlines. Avoid pills unless a future platform-native control specifically requires one.

## Components

### Buttons

- **Shape:** Compact and nearly square (4–6px radius), with a 44px minimum standard height.
- **Primary:** Service blue with white text and 11px × 18px padding; use once per immediate decision area.
- **Hover / Focus:** Darken to the service-blue hover token; preserve the shared 3px focus outline.
- **Secondary / Text:** Secondary buttons use a cool neutral fill and border. Text actions stay transparent and underline on hover.

### Chips

- **Style:** Small network and prototype tags use 3–4px corners, compact padding, explicit text, and a visible border.
- **State:** Tags communicate environment or provenance; they are not decorative badges and do not replace sentence-level status.

### Cards / Containers

- **Corner Style:** Functional data boxes use 4px corners; major screen regions have no radius.
- **Background:** Prefer the pale ground. Use paper white only for inputs, address blocks, and clearly bounded data.
- **Shadow Strategy:** None.
- **Border:** 1px hairlines provide all structural separation.
- **Internal Padding:** 13–24px according to data density and hierarchy.

### Inputs / Fields

- **Style:** Paper surface, strong 1px border, 4px radius, 46px minimum height.
- **Focus:** Border changes to service blue and gains the shared visible focus outline.
- **Error / Disabled:** Error language appears directly near the field or action in signal red. Disabled controls retain their label and use reduced opacity.

### Navigation

The compact horizontal navigation lives in the deep ink header. Labels use 0.8125rem semibold type; hover adds a subtle dark tonal fill, and the active destination uses white text plus a 3px light-blue underline. The network, node state, and explicit “Check node” action remain in the same status line.

### Amounts and Addresses

Balances are the largest element in the content region and use tabular numerals. Addresses appear as selectable monospace data with safe wrapping or deliberate ellipsis in lists; copy actions remain visibly labeled.

### Status and Errors

Statuses use concise, plain language such as “Connecting…”, “Height 123”, “Ready to use”, and “Node unavailable”. Errors state what failed and, where possible, what the user can do next. Never expose secrets or internal infrastructure details.

## Do's and Don'ts

### Do:

- **Do** make balance, current address, and next action immediately scannable.
- **Do** use hairlines and alignment to structure dense operational information.
- **Do** show transaction amounts, fees, recipients, and state changes explicitly before signing.
- **Do** keep status language calm, concrete, and independent of color.
- **Do** preserve selectable, wrapping monospace for addresses and identifiers.

### Don't:

- **Don't** introduce a sidebar-plus-card dashboard pattern.
- **Don't** use gradients, glass, soft shadow stacks, oversized radii, or decorative pills.
- **Don't** use monospace as a general aesthetic or service blue as decoration.
- **Don't** hide errors behind generic failure messages or use alarming promotional language.
- **Don't** transmit, display casually, or imply server custody of passwords, recovery phrases, seeds, or private keys.
