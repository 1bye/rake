# Plan: complete native tray support for Rake

Status: researched implementation plan; no tray implementation has landed.
Research date: 2026-11-05. Work phases in dependency order; each phase must leave
the existing application shapes and supported build configurations working.

Read sections 1–3 and 7 for the contract, sections 4–6 for the native API mapping,
and sections 13–14 for implementation order and acceptance gates.

- [Product contract](#1-product-contract-and-inventory-boundaries)
- [Existing Rake boundaries](#2-existing-rake-boundaries-to-preserve)
- [Capabilities and mapping rules](#3-mapping-rules-and-capability-vocabulary)
- [macOS API inventory](#4-macos-api-inventory-and-mapping)
- [Windows API inventory](#5-windows-api-inventory-and-mapping)
- [Linux API inventory](#6-linux-api-inventory-and-mapping)
- [Public API and ownership](#7-rake-public-api-and-ownership-design)
- [Menus and interaction](#8-menus-commands-and-interaction-semantics)
- [Rendering and accessibility](#9-images-text-animation-and-accessibility-polish)
- [GPUI popup windows](#10-rich-gpui-popup-windows)
- [Backend selection](#11-backend-and-dependency-decision)
- [Repository implementation map](#12-repository-implementation-map)
- [Delivery phases](#13-delivery-phases-and-acceptance-criteria)
- [Verification](#14-verification-and-platform-qualification)
- [Decisions and feasibility gates](#15-decisions-settled-by-this-plan-and-open-feasibility-gates)
- [Primary sources](#16-primary-sources-and-research-provenance)

## 1. Product contract and inventory boundaries

Provide a polished, cross-platform `rake::tray` API for macOS, Windows and Linux.
Support multiple independent items from the outset, both declared on `App` and
created dynamically. Include native menus, application commands, rich GPUI
popups, appearance updates, native specializations and observable lifecycle.

The feature target is the union of documented native capabilities, rather than
the intersection exposed by a convenient wrapper. A feature unavailable on one
host must remain accessible on another. Preserve OS conventions and user control.

This document inventories the public tray-specific surfaces, their relevant
menu/input/popup dependencies, legacy alternatives and newer documented APIs.
It does not claim to enumerate every inherited method on NSObject, NSView,
NSResponder, HWND, GObject or Xlib. Those general-purpose APIs are categorized
by their tray use below. Private shell protocols and undocumented menu-bar hacks
are excluded. Deprecated public tray APIs are explicitly accounted for.

The inventories describe **native availability**, not an implementation claim.
All Rake names/signatures here are proposed. Actual support requires the native
gates in section 14. A wrapped feature and a protocol feature can differ.

Required outcomes:

- Multiple items with stable IDs, independent menus/popups and clean teardown.
- Good rendering at every relevant scale, appearance and accessibility setting.
- Runtime updates without rebuilding the application or losing item identity.
- Native mouse and keyboard activation; semantic events plus native detail.
- Host disappearance/reappearance without stale items, busy polling or leaks.
- Correct behavior in routed, standalone, background and custom headless hosts.
- Clear unsupported results on browser hosts; no pretend browser system tray.
- No mandatory devtools, GTK, Tokio, updater or app-specific global variables.
- No artificial one-item limit or promise that the OS will display every item.

Adjacent systems stay separate: global shortcut registration, launch at login,
single-instance arbitration, modern OS notifications and the Dock/taskbar APIs.
Their integration points are recorded, but implementing them is not a prerequisite
for basic tray support. Legacy balloon APIs are inventoried because they are part
of native tray surfaces; they are not silently substituted for OS notifications.

## 2. Existing Rake boundaries to preserve

Read `AGENTS.md` and `.agents/workflow/rust/{AGENTS,architecture,stacks,testing,rustdoc}.md`
before implementation. Preserve Rust 1.98, edition 2024, the GPUI pins, additive
features and target-specific dependencies. Do not apply older version claims in
historical plans to this work.

| Existing owner | Reuse or necessary extension |
| --- | --- |
| `crates/rake/src/application.rs` | `App`, background launch, close policy and route type transitions; add declarative tray specifications and host startup validation. |
| `application/managed.rs` | Existing application-owned GPUI driver and quit observation; separate retained runtime ownership from a particular standalone root when implementing reopening. |
| `router/runtime.rs` and `router/runtime/windows.rs` | Routed window hosts and runtime lifetime; attach one tray host owner per application, never per route/window. |
| `runtime.rs` | Weak application capabilities; expose a weak tray collection without extending runtime lifetime. |
| `platform/mod.rs` and `services.rs` | Existing support/error vocabulary; share concepts, but do not put native GPUI resources in the portable service registry as their lifetime owner. |
| `control/commands.rs` and `control/mod.rs` | Typed application commands and explicit window requirements; dispatch through existing operations, independent of a development transport. |
| `quit.rs` and `router/runtime/quit.rs` | Guarded quit and finite work; built-in tray Quit uses this boundary. |
| `rake-build`, `rake-resources`, `rake-dist-core` | Existing assets, installation descriptors and bundle identity; embed tray identity/assets independently of enabling updates. |
| `rake-native` | Currently optional AppKit controls embedded in GPUI layouts; do not make desktop tray support depend on that macOS-only component scope. |
| `rake-ui`, existing overlays | Reuse GPUI rendering/theme utilities for a separate popup window; an in-window overlay cannot become a system tray popup by itself. |

Known prerequisites and constraints:

1. `Root` is currently `FnOnce`. `KeepRunning` closes a standalone window while
   its managed driver survives. Add an explicit repeatable window factory and
   reuse its runtime when a tray action recreates the main window. Do not call
   the current `managed::mount` unchanged and construct a second runtime.
2. `App::routes` changes the builder's route type. Transfer tray configuration
   across `routes`/`replace_routes` without making menu definitions route-generic.
3. GPUI foreground executor/app context is local. Native workers carry owned
   events/snapshots, not `Rc` services, borrowed GPUI state or UI closures.
4. AppKit resources live on the main thread after the host loop starts. Windows
   resources live on their owning message-loop thread. No second GUI loop.
5. Existing services/contexts can outlive shutdown. Every retained tray handle
   must fail safely when its application owner has stopped.
6. Current support documentation qualifies macOS source apps; Windows/Linux are
   unqualified. New API coverage must not quietly broaden that support claim.

## 3. Mapping rules and capability vocabulary

Each inventory entry has one destination:

- **Shared:** public Rake operation/event with host-dependent availability.
- **Extension:** typed `macos`, `windows` or `linux` options/operations.
- **Internal:** native implementation mechanism, hidden from application code.
- **Legacy:** documented compatibility surface with an explicit delivery phase.
- **Unavailable:** no supported native equivalent, or infeasible with the chosen
  host; return an error or expose an explicitly selected fallback.

Do not turn every platform option into a boolean in a large shared builder.
Use meaningful enums for visibility, icon rendering, primary interaction,
placement, menu presentation and removal behavior.

Distinguish these facts:

- Backend compiled and enabled.
- Native API/runtime version supports an operation.
- A compatible shell/panel host currently exists.
- Requested state was admitted locally, applied by the backend, or rejected.
- An item is registered/exported, the application requests it shown, and the
  shell actually presents it. The last fact may be unknowable.

`TrayCapabilities` should expose operations individually: multiple items,
normal/template/theme images, plain/rich tooltip, inline label, overlay,
attention, menu features, event families, rectangle/hint quality, popup placement,
user removal and persistent native identity. Use `Supported`, `Unsupported` and
`Unknown`/conditional evidence where appropriate. Include backend name, native
version requirements and reason without leaking backend types.

An explicit unsupported setter returns `TrayError::Unsupported` before changing
requested state. Best-effort appearance hints must be marked as hints in their
names/docs and return an effective application report when relevant. No silent
empty rectangle, dropped menu option or fake success. Capability queries are
snapshots, not guarantees that later native calls cannot fail.

## 4. macOS API inventory and mapping

Evidence: installed Xcode 26.6 macOS SDK headers `NSStatusBar.h`, `NSStatusItem.h`,
`NSStatusBarButton.h`, and Apple's current DocC symbol data/documentation.
The installed headers lack the newer macOS 27 expanded-interface symbols.
Compile-time SDK support and runtime OS availability must both be checked.
Sources: [status bar][mac-bar], [status item][mac-item], [button][mac-button],
[behavior][mac-removal], [expanded interface][mac-expanded].

### 4.1 Complete direct status bar/item surface

| Native API/symbol | Rake mapping and delivery |
| --- | --- |
| `NSStatusBar.system` / Objective-C `systemStatusBar` | Internal application-wide connection to the system bar. |
| `statusItem(withLength:)` / `statusItemWithLength:` | Create each item independently; `TrayCollection::create`. Multiple calls are supported. P2. |
| `removeStatusItem(_:)` | Explicit removal and best-effort owner teardown. P2. |
| `NSStatusBar.isVertical`, `thickness` | Read-only layout metrics in native observations; never a global panel layout setter. P5. |
| `NSStatusItem.statusBar` | Internal ownership/geometry context; no unrelated bar object in the shared API. |
| `length`, `squareLength`, `variableLength`, corresponding C length constants | `macos::ItemLength::{Square,Content,Points}`; reject invalid lengths; preserve logical point units. P5. |
| `menu` | Native menu attachment; automatic native tracking is distinct from custom activation/popup handling. P3. |
| `button` | Adapter access to the existing status bar button for supported appearance, action and accessibility configuration. P2/P5. |
| `behavior`, `NSStatusItem.Behavior`, raw-value construction | Typed macOS removal policy; retain unknown native flags internally rather than exposing a shared integer. P5. |
| `.removalAllowed` | Allow native user removal; observe `isVisible`, persist user choice, provide an application path to restore the item. P5. |
| `.terminationOnRemoval` | Native termination is a distinct, potentially unguarded behavior. Default never sets it. Prefer managed `RemovalPolicy::RequestQuit`; any explicit native-termination extension must document bypassing Rake's guards and pass a dedicated gate. P5. |
| `isVisible` / Objective-C `visible` | Requested visibility plus available KVO observation. This is not proof of unobscured on-screen pixels. P2/P5. |
| `autosaveName`, `NSStatusItem.AutosaveName` | Stable namespaced key per item; preserve native saved information. Never share one key between multiple items or clear it accidentally on ordinary teardown. P2. |
| `expandedInterfaceDelegate` | Newer native management of a custom expanded interface; map to the popup host when compiled/runtime available. Apple metadata says macOS 27.0. P6. |
| `expandedInterfaceSession` | Track the current native popup session separately from a GPUI window handle. P6. |
| `NSStatusItemExpandedInterfaceDelegate.statusItem(_:didBegin:)` | Begin/show the GPUI expanded window in the native session. P6. |
| `statusItemDidEndExpandedInterfaceSession(_:animated:)` | Dismiss with the native animation/lifecycle signal; release session resources. P6. |
| `NSStatusItemExpandedInterfaceSession.cancel()` | Programmatic dismissal through the system session. P6. |
| `NSStatusBarButton.appearsDisabled` | macOS visual disabled appearance independent of interaction; do not confuse with actual `isEnabled`. P5. |

The current Apple web index can label recently added symbols as beta. The
macOS 27 availability above comes from DocC metadata, not the local SDK or a
successful native run. Do not force an SDK upgrade or hard-link absent symbols.

### 4.2 Inherited button/image/input capabilities relevant to trays

| Public native surface | Rake mapping |
| --- | --- |
| `NSButton.image`, `alternateImage`, image position/scaling; `NSImage.isTemplate` and representations | Multi-resolution image assets; template rendering; alternate pressed image where reliable. Keep symbol/name image selection in a typed native image extension. |
| `title`, `attributedTitle`, `alternateTitle`, font/alignment/content tint where supported | Plain inline text and macOS attributed/native text styling. Text belongs to the status button, not a universal Windows tray label. |
| `isEnabled`, state, highlight configuration | Separate interaction enabled state, selection/highlight and disabled appearance. Explicitly define whether disabled native items still open menus. |
| `target`, `action`, `sendAction(on:)`, `performClick`, button double-action support | Adapter receives native actions; maps primary/secondary and optional click detail. Preserve actual click count; no timer-synthesized universal double-click contract. Programmatic activation is separately labeled. |
| `NSEvent` button, modifiers, timestamp, location, click count and scroll information | Rich native event detail when received. Tracking areas/subclass hooks require an integration spike; merely inheriting from NSView does not prove every event is delivered to status items. |
| `NSView.toolTip`, `NSAccessibility` label/help/value/role/action properties | Plain tooltip and accessible name/help. Keep useful labels when the icon is text-only or purely decorative. |
| `bounds`, `window`, coordinate conversion, backing scale, display notifications | Optional `TrayAnchor` with display and native/logical conversion metadata; invalidate when displays, scale or menu-bar configuration change. |
| Public dragging-destination registration/protocols and event tracking hooks | Advanced drag/drop and hover support only after a public-API feasibility experiment on the existing status button. No swizzling/private view replacement to manufacture support. Return unsupported if no sound host hook exists. P7. |
| Effective appearance, accessibility display preferences and image changes | Regenerate rendered assets only when inputs change; native template images should use OS styling. |

### 4.3 Deprecated direct NSStatusItem APIs, explicitly accounted for

All are inventoried rather than used as the normal implementation. Most direct
appearance/action properties were deprecated in macOS 10.14 in favor of `button`.

| Deprecated native names | Mapping |
| --- | --- |
| `action`, `doubleAction`, `target`, `sendActionOn:` | Corresponding modern button action/event bridge. |
| `title`, `attributedTitle`, `image`, `alternateImage`, `enabled`, `toolTip` | Corresponding button properties. |
| `highlightMode` | Native button/cell styling or the expanded-interface session; do not reproduce deprecated background painting. |
| `view` | Do not base rich Rake content on replacing the status item's view. Use the standard button plus a separate GPUI popup window. |
| `drawStatusBarBackgroundInRect:withHighlight:` | Let the supported native button/session render its highlight. |
| `popUpStatusItemMenu:` | Use supported menu attachment or modern NSMenu popup APIs. |

### 4.4 Complete menu feature families exposed by AppKit

Keep these capabilities available through a native menu implementation, even if
some are specialized. Sources: [NSMenu][mac-menu], [NSMenuItem][mac-menu-item],
[NSMenuDelegate][mac-menu-delegate], [popup][mac-popover].

| Native API family (symbol inventory) | Rake contract |
| --- | --- |
| `NSMenu.init(title:)`, `NSMenuItem.init(title:action:keyEquivalent:)`; coder initializers | Runtime menu construction. Archive/coder restoration stays internal/unneeded for the Rust descriptor format. |
| `addItem`, `insertItem`, `removeItem`, `removeAllItems`, `items`, `numberOfItems`, `itemChanged` | Stable-keyed menu trees and live insert/remove/update. Native index bookkeeping remains internal. |
| `item(at:)`, `item(withTag:)`, `item(withTitle:)`, all `index(of:)` / `indexOfItem` variants | Rake lookup by `MenuItemId`; native tag/title/target/represented-object/submenu lookups are adapter details. |
| `setSubmenu`, `submenuAction`, `supermenu`; item `submenu`, `hasSubmenu`, `parent`, `menu` | Nested submenus, parent relationships and cycle/duplicate validation. |
| Item `title`, `attributedTitle`, `subtitle`, `toolTip` | Shared plain label; typed macOS attributed label/subtitle/help. Subtitle is 14.4+; attributed-title/subtitle combination has OS-specific behavior. |
| `image`, `onStateImage`, `offStateImage`, `mixedStateImage`, `preferredImageVisibility`, `ImageVisibility` | Normal/menu-state images and native visibility policy. |
| `state`, `NSMenu.selectedItems`, `selectionMode`, `SelectionMode.{automatic,selectAny,selectOne}` | Off/on/mixed checks and radio/selection groups. Keep application model authoritative, validate groups and reconcile native changes. |
| `isEnabled`, `isHidden`, `isHiddenOrHasHiddenAncestor`, `autoenablesItems`, `update` | Explicit enabled/visible projection from command availability; choose controlled validation over accidental responder-chain defaults. |
| `separator`, `isSeparatorItem`, `sectionHeader(title:)`, `isSectionHeader` | Separators and section headings, with explicit fallback on hosts lacking headings. Account for the deprecated `sectionHeader(withTitle:)` spelling. |
| `badge`, `NSMenuItemBadge` | Native menu-item badge (14.0+); map `init(count:)`, `init(string:)`, `alerts(count:)`, `newItems(count:)`, `updates(count:)` and `BadgeType` to typed count/text/style options; `itemCount`, `stringValue`, `type` are observations. Do not confuse with a tray-icon badge. |
| `keyEquivalent`, `keyEquivalentModifierMask`, `usesUserKeyEquivalents`, `userKeyEquivalent`, `allowsAutomaticKeyEquivalentLocalization`, `allowsAutomaticKeyEquivalentMirroring`, `allowsKeyEquivalentWhenHidden` | Menu-scoped accelerators and native override policy; never implicitly global shortcuts. |
| `isAlternate`, `indentationLevel`, `tag`, `representedObject`, `target`, `action`, `isHighlighted`, menu `highlightedItem` | macOS alternate items/indentation and observed highlight; keep identity and command payloads in Rake storage. |
| `NSMenuItem.view` | Optional advanced native menu content with accessibility/lifecycle proof; arbitrary GPUI elements default to a popup, not an NSMenu item. |
| Menu `font`, `minimumWidth`, `size`, `title`, `showsStateColumn`, `userInterfaceLayoutDirection` | Native menu styling/layout extension, read-only measurements and RTL behavior. |
| `presentationStyle`, `PresentationStyle.{regular,palette}`, `palette(colors:titles:template:onSelectionChange:)` | Native palette/menu presentation (14.0+); specialized color/selection menu descriptors. |
| `popUp(positioning:at:in:)`, both `popUpContextMenu` variants, `cancelTracking`, `cancelTrackingWithoutAnimation` | Explicit show/dismiss operations, placement and cancellation. Main-thread tracking must not deadlock Rake's event driver. |
| `performKeyEquivalent`, `performActionForItem` | Explicit menu activation/testing bridge; preserve enabled-state checks and dispatch exactly once. |
| `propertiesToUpdate`, `NSMenu.Properties`; delegate `menuNeedsUpdate`, `numberOfItems`, `menu(_:update:at:shouldCancel:)` | Fast cached menu preparation/validation and incremental population. Property bits cover item title, attributed title, key equivalent, image, enabled state and accessibility description; never wait for remote I/O inside a native delegate. |
| Delegate `menuWillOpen`, `menuDidClose`, `menu(_:willHighlight:)`, `menuHasKeyEquivalent`, `confinementRect` | Open/close/highlight events, native keyboard dispatch and constrained placement extensions. |
| `didAddItemNotification`, `didChangeItemNotification`, `didRemoveItemNotification`, `didBeginTrackingNotification`, `didEndTrackingNotification`, `willSendActionNotification`, `didSendActionNotification` | Internal observation; expose useful menu lifecycle events rather than duplicate two public event systems. |
| `NSMenu.DidAddItemMessage`, `DidChangeItemMessage`, `DidRemoveItemMessage`, `DidBeginTrackingMessage`, `DidEndTrackingMessage`, `WillSendActionMessage`, `DidSendActionMessage` | Availability-gated typed notification equivalents; the same Rake lifecycle mapping as the corresponding notifications. |
| `allowsContextMenuPlugIns`, `automaticallyInsertsWritingToolsItems`, item `writingToolsItems`, `importFromDeviceIdentifier` | Specialized native system integration. Default Rake tray menus remain application-defined; opt-in platform options, not silent automatic insertions. |
| `menuBarVisible`, `setMenuBarVisible`, `menuBarHeight`; deprecated global menu management | Not item configuration. Reading metrics can inform popup placement; changing global menu-bar visibility is outside the tray API. |
| Deprecated `setTitleWithMnemonic`, older NSMenu symbols | Use explicit text/access-key model and modern equivalents; no new dependency on deprecated menu management. |

`NSPopover` is an alternative native container, not a prerequisite for GPUI
popups: inventory `contentViewController`, `contentSize`, `positioningRect`,
`show(relativeTo:of:preferredEdge:)`, `behavior`, appearance/effective appearance,
animation, shown/detached state, `close`/`performClose`, delegate and show/close
notifications. Compare it to an NSPanel/GPUI window in P6. Additional newer popup
overloads/full-size-content options must be availability-gated. A native popover
does not automatically know how to host GPUI's renderer.

## 5. Windows API inventory and mapping

Sources: [Shell_NotifyIconW][win-notify], [NOTIFYICONDATAW][win-data],
[NOTIFYICONIDENTIFIER][win-id], [icon rectangle][win-rect],
[notification area][win-area], [taskbar recreation][win-taskbar].
Use Unicode APIs and the Windows versions supported by the pinned GPUI adapter;
do not promise XP/Vista support because a Win32 page lists those historical minima.
`Shell_NotifyIconA` and encoding-neutral aliases represent the same operations;
the adapter uses `Shell_NotifyIconW` exclusively rather than duplicating an ANSI API.

### 5.1 Direct shell operations, structures and all tray flags

| Native API/field/flag | Rake mapping |
| --- | --- |
| `Shell_NotifyIconW(NIM_ADD)` | Register one item; restore all desired items after shell recreation. P2. |
| `NIM_MODIFY` | Apply icon, tooltip, visibility and native balloon changes. P2/P7. |
| `NIM_DELETE` | Remove one item and release adapter-owned resources safely. P2. |
| `NIM_SETFOCUS` | Return keyboard focus to the notification area after menu/popup interaction where appropriate. P3/P6. |
| `NIM_SETVERSION`, `NOTIFYICON_VERSION_4` | Negotiate modern keyboard/accessibility callback behavior after every add, including recovery. Internal; keep legacy callback decoding separate if needed. |
| `NOTIFYICONDATAW.cbSize` | Correct initialized structure layout; internal ABI concern. |
| `hWnd`, `uID`, `uCallbackMessage` | App-owned hidden callback window and ID routing; independent of the main visible window. |
| `guidItem`, `NIF_GUID` | Stable Windows identity derived from app/profile/item identity; optional explicit GUID override. Never confuse with one shared application GUID. |
| `uFlags`: `NIF_MESSAGE`, `NIF_ICON`, `NIF_TIP`, `NIF_STATE` | Internal selective updates for callback, image, tooltip and state. |
| `NIF_SHOWTIP` | Standard tooltip policy in version-4 mode; custom hover UI is explicit. |
| `hIcon` | Properly sized icon representation and owned/shared native handle management. |
| `szTip` | Plain tooltip; validate UTF-16 capacity (127 usable code units in the modern 128-unit buffer), interior NUL and Unicode boundaries. Explicit error or documented selected truncation; no accidental byte truncation. |
| `dwState`, `dwStateMask`, `NIS_HIDDEN` | Application-requested hide/show. Not control over overflow promotion or user pinning. |
| `NIS_SHAREDICON` | Native resource-sharing flag; internal resource ownership optimization, not shared public tray identity. |
| `uVersion` | Internal protocol negotiation, not an arbitrary public integer knob. |
| `NOTIFYICONIDENTIFIER.{cbSize,hWnd,uID,guidItem}` | Native geometry lookup identity; multiple `uID`s per owner HWND. |
| `Shell_NotifyIconGetRect` | `TrayHandle::anchor` returns a fallible optional/qualified geometry result with physical screen units. |

Modern callbacks pack a **16-bit item ID** in `HIWORD(lParam)`; `LOWORD(lParam)`
contains the event code. Decode signed anchor coordinates from `wParam` with
the documented event validity rules. Never treat packed data as the old
`wParam = item ID` format. Allocate IDs without routing old queued callbacks to
a newly created item. Exhaustion must return an error or allocate another owner
window; it must not wrap silently.

### 5.2 Full callback/event families relevant to tray interaction

| Native callback/messages | Rake mapping and limitations |
| --- | --- |
| `NIN_SELECT`, `NIN_KEYSELECT` | Semantic activation, preserving keyboard versus pointer origin; deduplicate overlapping low-level mouse notifications. |
| `WM_CONTEXTMENU` | Context-menu request from pointer or keyboard; keyboard activation must work without inventing a mouse button. |
| Delivered `WM_MOUSEMOVE`, left/right/middle and X-button down/up/double-click messages | Optional native pointer event detail; characterize actual shell delivery. Do not claim every Windows mouse message is forwarded by every shell. |
| `NIN_POPUPOPEN`, `NIN_POPUPCLOSE` | Rich hover UI/popup lifecycle hints. They are not a universal raw mouse-enter/leave contract. |
| `NIN_BALLOONSHOW`, `NIN_BALLOONHIDE`, `NIN_BALLOONTIMEOUT`, `NIN_BALLOONUSERCLICK` | Legacy tray-balloon observation only; keep separate from modern notification events. |
| `RegisterWindowMessageW("TaskbarCreated")` | Re-register desired items and version after Explorer/taskbar restart or other documented broadcasts; replay latest state once. |
| `WM_SETTINGCHANGE`, `WM_THEMECHANGED`, display/DPI/session/shutdown messages as applicable | Invalidate image/layout state, observe lifecycle and clean up. Verify which messages the chosen owner HWND receives. |

TaskbarCreated is broadcast to **top-level windows**. Do not rely solely on
`HWND_MESSAGE`, which does not receive ordinary broadcasts. Reuse a reliable
GPUI hook or own a hidden top-level callback HWND on its existing message loop.
Avoid replacing a GPUI window procedure or using a visible main window as the
tray's only owner. The owner must survive closing all application windows.

No documented portable Shell_NotifyIcon tray API guarantees wheel events,
drag/drop targets, inline text, forced ordering/pinning, exact visible-state
queries or per-item native attention overlays. Mark these unavailable on this
backend unless a public, tested route is established. Composited image badges
are an explicit Rake rendering feature; taskbar-button overlays are a different API.

### 5.3 Legacy balloon fields and controls, fully accounted for

| Native surface | Mapping |
| --- | --- |
| `NIF_INFO`, `szInfo`, `szInfoTitle` | Optional `windows::LegacyBalloon` API tied to a tray item. Validate 255/63 usable UTF-16 units in the native buffers and preserve encoding. |
| `NIF_REALTIME` | Explicit discard-if-stale delivery policy. |
| `dwInfoFlags`: `NIIF_NONE`, `NIIF_INFO`, `NIIF_WARNING`, `NIIF_ERROR`, `NIIF_USER` | Typed legacy balloon icon selection. |
| `NIIF_NOSOUND`, `NIIF_LARGE_ICON`, `NIIF_RESPECT_QUIET_TIME` | Explicit sound/icon/quiet-time options; sensible quiet-time default. |
| `NIIF_ICON_MASK` | Reserved mask/internal decoding; no public application option. |
| `hBalloonIcon` | Separate custom balloon icon ownership. |
| Union `uTimeout` | Deprecated/ignored by modern Windows timing policy; document, do not promise user-selected display duration. |
| Empty balloon body / item removal | Explicit legacy withdrawal; no guarantee a previously delivered notification becomes unseen. |

These are P7 compatibility features. A modern toast/notification system stays
independent; sending a tray balloon does not establish cold activation support.

### 5.4 Native popup menu inventory

Sources: [menu model][win-menus], [MENUITEMINFOW][win-menu-item],
[MENUINFO][win-menu-info], [TrackPopupMenuEx][win-popup].

| Native surface | Rake mapping |
| --- | --- |
| `CreatePopupMenu`, `DestroyMenu`; `LoadMenuW`, `LoadMenuIndirectW` | Owned runtime menus; native resource-template import is an optional Windows extension, not the shared descriptor format. |
| `InsertMenuItemW`, `SetMenuItemInfoW`, `GetMenuItemInfoW`, `RemoveMenu`, `DeleteMenu`; older append/insert/modify APIs | Stable-keyed item/tree updates; choose modern APIs internally while accounting for equivalent older entry points. |
| `GetMenuItemCount`, `GetMenuItemID`, `GetSubMenu`, `GetMenuState`, `GetMenuStringW`, `IsMenu` | Native lookup/query details behind Rake IDs and snapshots. |
| `MENUITEMINFO` ID, submenu, text, item-data and bitmap/checkmark fields; `MIIM_*` masks | Text, nested menus, images, off/on images, command bindings; native pointers never become public Rake IDs. |
| `MFT_STRING`, `MFT_SEPARATOR`, `MFT_RADIOCHECK`; `MFS_CHECKED/UNCHECKED`, enabled/disabled/grayed/default/highlighted state | Shared menu kinds, checks/radio groups, availability and default item; read back native state only where meaningful. |
| `MFT_OWNERDRAW`, `WM_MEASUREITEM`, `WM_DRAWITEM`, bitmap items | Windows advanced custom drawing with explicit accessibility/theme/scale gate. Prefer GPUI popup for general interactive controls. |
| `MFT_MENUBREAK`, `MFT_MENUBARBREAK`, `MFT_RIGHTJUSTIFY`, `MFT_RIGHTORDER`, legacy `MFT_BITMAP` | Native layout extensions or obsolete equivalents, not universal menu presentation guarantees. |
| `CheckMenuItem`, `CheckMenuRadioItem`, `EnableMenuItem`, `Set/GetMenuDefaultItem`, `HiliteMenuItem`, `SetMenuItemBitmaps` | Shared state/group/default-selection operations; avoid duplicate dispatch and model drift. |
| `TrackPopupMenu` / `TrackPopupMenuEx`, `TPMPARAMS.rcExclude`, `CalculatePopupWindowPosition` | Native context-menu display with anchor exclusion, screen clamping and consistent physical coordinates. |
| `TPM_LEFT/CENTER/RIGHTALIGN`, `TOP/VCENTER/BOTTOMALIGN`, `HORIZONTAL/VERTICAL`, `LAYOUTRTL`, `WORKAREA` | Typed placement/direction policy or internal placement mechanics. |
| `TPM_LEFTBUTTON/RIGHTBUTTON`, `RETURNCMD`, `NONOTIFY`, `RECURSE`, animation flags, `NOANIMATION` | Typed invocation/animation policy; select one command return/message path. Honor reduced animation. Validate nested tracking instead of allowing arbitrary reentrancy. |
| `SetForegroundWindow`, post-tracking focus handling and `NIM_SETFOCUS` | Proper dismissal and keyboard focus restoration; follow documented foreground constraints, no focus stealing from background updates. |
| `EndMenu`, `GetMenuItemRect`, `MenuItemFromPoint`, `GetMenuCheckMarkDimensions`, `GetMenuInfo`/`SetMenuInfo` | Explicit close/read-only geometry and adapter layout; expose only useful stable native extension operations. |
| `MENUINFO` style, maximum height, background brush, context help and menu data | Windows extension for menu presentation/help; owned native resources and data remain internal. |
| `MNS_AUTODISMISS`, `CHECKORBMP`, `NOCHECK`, `MODELESS`, `NOTIFYBYPOS`, `DRAGDROP` | Advanced native options; normal menus keep OS defaults. Modeless/drag menus require their own lifetime and accessibility gates. |
| `WM_INITMENU`, `WM_INITMENUPOPUP`, `WM_UNINITMENUPOPUP`, `WM_ENTERMENULOOP`, `WM_EXITMENULOOP`, `WM_MENUSELECT` | Cached preparation, open/close/highlight events and modal tracking observation. |
| `WM_COMMAND`, `WM_MENUCOMMAND`, `WM_MENUCHAR`, `WM_HELP`, `WM_MENURBUTTONUP`, `WM_MENUDRAG`, `WM_MENUGETOBJECT` | Selection/mnemonic/help and optional native drag integration; never assume a tray icon itself becomes an OLE drop target. |
| Accelerator tables / `TranslateAcceleratorW` | Separate menu/window-scoped keyboard bridge. A displayed shortcut alone does not install accelerator processing or a global shortcut. |

## 6. Linux API inventory and mapping

Linux has several public tray protocols and host implementations. The primary
backend should be StatusNotifierItem plus DBusMenu, working on X11 and Wayland
when a compatible host exists. Inventory both `org.freedesktop.*` specification
names and deployed `org.kde.*` names; negotiate/implement compatibility deliberately.
Do not assume two names, object paths or property types are interchangeable.

Sources: [SNI item specification][sni-item], [watcher][sni-watcher],
[deployed SNI XML][sni-xml], [deployed watcher XML][watcher-xml].

### 6.1 All StatusNotifierItem properties, requests and signals

| Native protocol member | Rake mapping |
| --- | --- |
| `Id` | Stable app/profile/item identity; native unique bus/object identity remains separately allocated. |
| `Category` | `linux::Category::{ApplicationStatus,Communications,SystemServices,Hardware}`. Ayatana additionally defines `Other`; do not send it to a strict host without compatibility handling. |
| `Title` | Descriptive item title/metadata. Not a guaranteed inline panel label. |
| `Status`: `Passive`, `Active`, `NeedsAttention` | Linux status extension. Passive is a host presentation hint, not guaranteed removal or invisibility. |
| `WindowId` | Optional associated native X11 window; use zero/absence on Wayland instead of casting a foreign window handle. Deployed XML uses signed `i`, specification uses unsigned; adapter must account for that. |
| `IconName`, `IconPixmap` | Named theme icon and/or multi-resolution image data; define precedence and explicit fallback. Convert to the required ARGB32 wire encoding. |
| `OverlayIconName`, `OverlayIconPixmap` | Native overlay extension; actual rendering remains host-controlled. |
| `AttentionIconName`, `AttentionIconPixmap`, `AttentionMovieName` | Attention image/movie hints and status. Movie support is conditional; no promise that a host animates it. |
| `ToolTip` tuple | Rich tooltip icon/title/body. Sanitize/limit supported markup; no arbitrary HTML/GPUI promise. |
| `ItemIsMenu`, `Menu` | Menu-primary interaction intent and DBusMenu object path. Hosts may own context-menu rendering and right-click behavior. |
| `IconThemePath` (deployed extension) | Typed Linux icon search-path option with explicit resource lifetime. |
| `Activate(x,y)` | Semantic primary activation with a coordinate hint of known quality, not an exact icon rectangle. |
| `SecondaryActivate(x,y)` | Semantic secondary activation, commonly middle button; do not rename it right-click. |
| `ContextMenu(x,y)` | Host context-menu request. Avoid opening a duplicate local menu when the host already renders the exported DBusMenu. |
| `Scroll(delta,orientation)` | Horizontal/vertical scroll event with native units/source; do not claim pixel or detent equivalence to other backends. |
| `ProvideXdgActivationToken(token)` (deployed extension) | Capture activation context and carry it with the following event into GPUI focus/popup handling. Treat it as short-lived opaque data. |
| `NewTitle`, `NewIcon`, `NewAttentionIcon`, `NewOverlayIcon`, `NewToolTip`, `NewStatus(status)` | Emit appropriate native update signals after state changes; coalesce safely without losing final state. |

The native `IconPixmap` can carry multiple representations. Separate icon
resolution selection from animation; do not interpret every multi-image vector
as animation. Confirm byte order, alpha convention and dimensions with actual
hosts using a colored/transparent diagnostic asset.

### 6.2 All watcher/host lifecycle members

| Native member | Rake mapping |
| --- | --- |
| `RegisterStatusNotifierItem(service-or-object-path)` | Register every item after export; track bus identity, ownership and registration result. |
| `RegisteredStatusNotifierItems` | Optional registration evidence, not actual pixel visibility. |
| `IsStatusNotifierHostRegistered`, `ProtocolVersion` | Host availability/version snapshot. Protocol version is not a menu feature negotiation mechanism. |
| `StatusNotifierItemRegistered`, `StatusNotifierItemUnregistered` | Observe registration state relevant to our items. |
| `StatusNotifierHostRegistered`, deployed `StatusNotifierHostUnregistered` | Update availability and re-export/re-register where needed. |
| `RegisterStatusNotifierHost` | Host-side operation; inventory only. Rake is an item producer, not a replacement desktop panel. |
| D-Bus name ownership changes, connection loss, Properties/Introspectable interfaces | Internal reconnect/recovery, protocol introspection and state queries; bounded retries and explicit shutdown. |

No host is a normal environmental state. Report `WaitingForHost`/`HostUnavailable`
without claiming an item is visible. Do not install extensions or start a panel.
With fallback selection enabled, register only one backend per item at a time;
host arrival must not create duplicate SNI and XEmbed icons.

### 6.3 All DBusMenu protocol members and item properties

Source: [documented DBusMenu XML carried by KSNI][dbusmenu]. Rake owns the menu
model; use an existing implementation where it meets the contract.

| Protocol member | Rake mapping |
| --- | --- |
| Root properties `Version`, `TextDirection`, `Status`, `IconThemePath` | Native version, RTL, menu-level status and theme paths; keep distinct from SNI item status. |
| `GetLayout(parentId,recursionDepth,propertyNames)` | Revisioned bounded menu tree projection; root ID zero; validate depth and requested properties. |
| `GetGroupProperties`, `GetProperty` | Efficient cached property snapshots and standard defaults. |
| `Event`, `EventGroup` | Validate item ID, event type, data and timestamp; dispatch supported clicked/hovered/opened/closed events according to the protocol. No arbitrary command execution from unrecognized payloads. |
| `AboutToShow`, `AboutToShowGroup` | Quick preparation from cached app state; report changed subtree/revision. Native opening cannot wait indefinitely for a GPUI future or network call. |
| `ItemsPropertiesUpdated`, `LayoutUpdated` | Batch property changes and invalidate only affected layout subtrees. |
| `ItemActivationRequested(id,timestamp)` | Best-effort request for the host to activate a menu item/menu; not guaranteed forced panel opening. |
| Item `type`, `label`, `enabled`, `visible` | Standard/separator types, text and availability. Escape literal underscores and encode access keys correctly. |
| `icon-name`, `icon-data` | Named icon or PNG menu image; this differs from SNI's raw ARGB pixmap encoding. |
| `shortcut` | Menu shortcut metadata, including supported sequences/modifiers; no automatic system-wide registration. |
| `toggle-type`, `toggle-state` | Checkmark/radio and off/on/indeterminate states. Rake enforces radio exclusivity; the protocol does not. |
| `children-display` | Submenu presentation plus exported children. |
| `disposition` | Normal/informative/warning/alert hint; host may choose presentation. |
| `x-<vendor>-*` properties/types | Typed, documented extensions only where mapped. No blanket guarantee that arbitrary custom widget types render on every desktop. |

DBusMenu does not expose generic arbitrary GPUI content, exact tray geometry,
raw pointer enter/leave, or a reliable universal menu-tracking barrier. A menu
preparation callback may arrive late on some hosts; maintain an already usable
snapshot rather than depending on it for correctness.

### 6.4 Ayatana/AppIndicator public API variants

These are alternative libraries/extensions on top of Linux indicator protocols,
not capabilities that every SNI host implements. The inventory checks upstream
headers, not just older generated documentation. Sources: [GTK-based header][aya-gtk],
[GLib replacement header][aya-glib], [replacement project][aya-project].

| Public C API family (prefix `app_indicator_`) | Mapping |
| --- | --- |
| `get_type`, GObject type/cast checks; `new`, `new_with_path` | Internal type/constructor mechanics; supplied stable ID, icon/category and optional theme path. Constructor deprecation varies by library snapshot. |
| `set_status` / `get_status`, category/ID getters | Linux status/category/identity. |
| GTK `set_icon`, `set_icon_full`, `get_icon`, `get_icon_desc`; GLib `set_icon` with description | Normal image plus accessible description. Keep version-specific signatures inside the adapter. |
| GTK `set_attention_icon`, `set_attention_icon_full`, getters; GLib description-taking variant | Attention image and accessible description. |
| `set_title` / `get_title` | Metadata title. |
| `set_icon_theme_path` / getter | Native theme path/resource ownership. |
| `set_label(label,guide)`, `get_label`, `get_label_guide` | Linux inline-label extension and width-stabilizing guide; host-dependent presentation. |
| `set_ordering_index` / getter | Linux ordering hint; cannot force a global position. |
| `set_secondary_activate_target` / getter | Typed secondary-action binding; GTK targets a widget, GLib replacement targets an action name. Rake binds a command. |
| `set_menu` / `get_menu` | GTK uses `GtkMenu`; newer GLib API uses `GMenu`. Keep native type differences private. |
| GLib `set_actions` / `get_actions` | `GSimpleActionGroup` bridge to commands/menu state, if that backend is adopted. |
| GTK `set_tooltip_icon/title/body/full`, matching getters; GLib `set_tooltip` | Rich tooltip where the actual installed library supports it. These upstream additions must not be claimed for older distributions. |
| GTK `build_menu_from_desktop` | Optional native desktop-action import. Do not read/execute arbitrary .desktop commands through a generic menu API. |
| Signals `new-icon`, `new-attention-icon`, `new-status`, `new-icon-theme-path`, `new-label`, `new-tooltip`, `connection-changed`, `scroll-event`, newer `activate` | State/host/input bridge; signal availability depends on release. |
| GTK subclass virtual `fallback`, `unfallback` | Legacy fallback mechanics; not application extension points in Rake. Own fallback arbitration centrally. |
| Native wire properties `IconAccessibleDesc`, `AttentionAccessibleDesc`, `XAyatanaLabel`, `XAyatanaLabelGuide`, `XAyatanaOrderingIndex` | Explicit accessibility/label/ordering extensions. |
| Wire `XAyatanaSecondaryActivate`, `XAyatanaNewLabel`, `NewIconThemePath` | Compatibility activation/update messages when actually supported. |

The GTK-based upstream marks itself obsolete in favor of a GTK-free GLib
implementation. That replacement is **not** the same as KSNI or DBusMenu; it
uses GMenu/actions and its public header declares GPL-3.0 licensing. Treat it as
an inventory/reference candidate, not an automatically approved dependency for
this MIT/Apache workspace. Select dependencies using their actual release,
license, ABI, system-library and runtime requirements at P0.

### 6.5 Legacy X11 system tray and XEmbed

Support is useful for X11-only legacy panels. It is not native Wayland support,
and running under XWayland does not guarantee an XEmbed tray manager exists.
Sources: [system tray protocol][xtray], [opcode messages][xtray-opcodes],
[manager hints][xtray-manager], [XEmbed][xembed].

| Native atom/API/message family | Rake mapping |
| --- | --- |
| `_NET_SYSTEM_TRAY_Sn`, selection-owner lookup, `MANAGER` announcement | Discover/observe a manager per X screen and recover when replaced. |
| `_NET_SYSTEM_TRAY_OPCODE`, `SYSTEM_TRAY_REQUEST_DOCK` | Dock one native X window per item. Explicit item lifecycle and multiple-item support. |
| XEmbed `_XEMBED`, `_XEMBED_INFO`, embedding/focus/mapping messages | Internal embedded-window lifecycle and focus behavior. Implement the client role only, not a tray manager. |
| `XEMBED_EMBEDDED_NOTIFY`, window activation/deactivation, focus in/out/request, traversal, modality and accelerator messages | Categorize supported XEmbed client behavior and test panel interoperability. Accelerator messages remain local embedding integration, not global shortcuts. |
| Configure/map/unmap/destroy/reparent/property events | Resize/repaint, availability and removal/recovery; observe manager loss without dereferencing stale XIDs. |
| `_NET_SYSTEM_TRAY_VISUAL`, ARGB/compositing and legacy ParentRelative behavior | Correct transparency, visual/depth selection and panel background handling. |
| `_NET_SYSTEM_TRAY_ORIENTATION` | Orientation/layout hint. |
| `WM_NORMAL_HINTS`, base/minimum size and aspect-ratio hints | Reasonable tray sizing; manager controls final geometry. |
| X pointer/button/key/enter/leave events on the embedded window | Richer legacy input with explicit event capability; never infer equivalent SNI events. |
| `_NET_SYSTEM_TRAY_MESSAGE_DATA`, `SYSTEM_TRAY_BEGIN_MESSAGE`, `SYSTEM_TRAY_CANCEL_MESSAGE` | Optional legacy balloon extension with bounded payload/reassembly rules; P7. |
| Panel color/size/padding extensions such as `_NET_SYSTEM_TRAY_COLORS`, `_NET_SYSTEM_TRAY_ICON_SIZE`, `_NET_SYSTEM_TRAY_PADDING` | De facto host hints, not guaranteed by the base protocol. Inventory exact supported variants against real panel implementations before enabling each. |

Do not bundle a new GTK event loop solely to obtain an XEmbed icon. First test a
small adapter over already-used X11 facilities or a suitable compatible library.
If adoption fails, document XEmbed as a named unsupported legacy backend rather
than claiming SNI covers every possible X11 panel.

## 7. Rake public API and ownership design

### 7.1 Public surface

Start with modules in `crates/rake`, behind an additive `tray` feature. The
feature enables GPUI and only the native dependencies needed on each target.
On WASM, the same descriptors/capability/error paths can compile, but creation
reports unsupported. Pure runtime builds without this feature remain unchanged.

Expose concrete types through `rake::tray`, with explicit exports:

| Type/API | Contract |
| --- | --- |
| `Tray` (specification builder) | Native-resource-free desired configuration. Validated before registration; no host work during builder construction. |
| `TrayId` | Stable application-scoped item name. App identity/profile namespaces are separate from native callback identity. |
| `TrayCollection` | Weak access to one application host registry; create/get/list/remove. Not a new portable runtime owner. |
| `TrayHandle` | Cheap local control reference to one registration generation; dropping it does not remove the item. Removal/shutdown makes methods fail predictably. |
| `TrayState` | Revisioned requested/applied configuration, registration/host state and qualified presentation knowledge; no fabricated visibility. |
| `TrayCapabilities` | Backend/version/host-dependent operation support and reasons. Include menu and geometry capabilities. |
| `TrayImage`, `TrayTooltip` | Validated image sources/representations and plain/rich tooltip data; platform specializations remain typed. |
| `TrayEvent`, `TrayEventContext` | Semantic event, source, registration identity, optional native detail, timestamp and activation/placement context. |
| `TrayAnchor` | Rectangle or point hint with coordinate space, display and confidence; can be unavailable. |
| `TrayInteraction` | Explicit primary/context/secondary policies: menu, popup, command or event callback where supported. |
| `TrayError` | Nameable, inspectable library failures with original causes/native codes where available. |
| `tray::macos`, `tray::windows`, `tray::linux` | Typed native options and advanced operations, with support/availability docs. |
| Native menu model | Stable `MenuId`/`MenuItemId`, common tree/state and platform options. Begin within the tray owner; extract a shared `rake::menu` surface only when a second consumer uses its contract. |

Use `#[non_exhaustive]` where future variants are expected, derive appropriate
standard traits, and avoid exposing `tray_icon`, `muda`, `ksni`, GTK, HWND or
AppKit types in common public signatures. Local UI handles need not be `Send`;
do not widen them with unsafe implementations to satisfy a generic guideline.

### 7.2 Declarative and dynamic examples

The following is design pseudocode, not a compiling promise. Use ordinary
builders/functions; no new macro is required for menus or tray setup.

```text
App::new("com.example.utility")
    .window_factory(mount_main_window)
    .close_policy(ClosePolicy::KeepRunning)
    .tray(
        Tray::new("main")
            .icon(assets.app_icon())
            .tooltip("Example Utility")
            .menu(main_menu())
            .primary(TrayInteraction::command(OpenMainWindow)),
    )
    .tray(
        Tray::new("network")
            .icon(assets.network_icon())
            .menu(network_menu())
            .macos(|native| native.inline_text("12 MB/s"))
            .linux(|native| native.category(Category::SystemServices)),
    );
```

Replace the illustrative pseudocode above with a tested `no_run` consumer during
P1. This plan's examples are intentionally uncompiled.

Proposed operation shapes:

```text
RuntimeContext::trays() -> Result<TrayCollection, TrayError>
TrayCollection::get(&TrayId) -> Result<TrayHandle, TrayError>
TrayCollection::items() -> owned snapshot/iterator of current item references
TrayCollection::create(Tray, &mut gpui::App) -> Request<TrayHandle>
TrayHandle::state() -> Result<TrayState, TrayError>
TrayHandle::capabilities() -> Result<TrayCapabilities, TrayError>
TrayHandle::set_icon(TrayImage, &mut gpui::App) -> Request<AppliedTrayState>
TrayHandle::set_tooltip(TrayTooltip, &mut gpui::App) -> Request<AppliedTrayState>
TrayHandle::set_menu(Menu, &mut gpui::App) -> Request<AppliedTrayState>
TrayHandle::show/hide/remove(&mut gpui::App) -> Request<AppliedTrayState>
TrayHandle::anchor(&mut gpui::App) -> Request<Option<TrayAnchor>>
TrayHandle::show_menu/show_popup/close_popup(..., &mut gpui::App) -> Request<...>
```

Requests are **initiated** on the foreground thread, matching Rake's existing
platform request convention. They own their state and never retain a borrowed
`gpui::App` across await. Native operations that finish synchronously return an
already-ready request; Linux transport operations can finish asynchronously.
Success describes the explicit completion boundary in the returned state, not
that a panel painted pixels. Do not use generic `Operation` merely to imitate
router navigation if a platform-style `Request` already solves this contract.

At P1 choose concrete names and whether `AppliedTrayState` needs a separate
receipt type. Preserve these semantics whichever spelling wins:

- `create` completes after native item export/creation and the selected host
  policy is resolved. `WaitForHost` may return a usable waiting item; `RequireHost`
  fails after a bounded attempt. Neither waits forever on a user interface.
- A failed create releases its reserved ID and partially created resources.
  A cancelled observation cannot undo a native creation already committed;
  ensure that the item is recoverable by its known ID and state is observable.
- Updates have monotonic revisions; a receipt identifies the revision applied
  or superseded. A later update never gets overwritten by an older completion.
- Unsupported requests fail before mutation. Partial multi-property native
  failures return the actual resulting state; do not claim universal rollback.
- Removing an item is terminal for that handle generation. Reusing its stable
  name yields a new generation, never revives old handles or pending callbacks.
- `remove` is idempotent for the same terminal registration; other operations
  on removed handles report `Removed`. Runtime shutdown reports `Stopped`.
- Removing/hiding an item closes its popup and cancels item animation. Replacing
  a menu while open uses the defined tracking policy, not unsafe immediate frees.
- Dropping a request cancels observation where possible, not committed native
  changes. Document which expensive preparation work can actually be cancelled.

### 7.3 Configuration, errors and unsupported behavior

Validate IDs, duplicate registrations, missing assets, invalid image dimensions,
overflowing image buffer sizes, non-finite sizes, native string limits/interior
NUL, menu cycles/depth, duplicate item IDs and conflicting primary interactions.
Set documented bounds for per-item image memory, menu node/depth count, pending
operations and callback queue depth. Choose limits from real use cases in P1,
not accidental integer widths. Return recoverable errors at native limits.

Error families: invalid specification, duplicate ID, missing/removed item,
unsupported feature, host unavailable, native failure, queue/admission limit,
timeout, cancelled observation and stopped host. Separate a backend's inability
to observe actual presentation from an operation failure.

Expose explicit startup policies for optional/waiting versus required tray
integration. Required native creation errors join `StartupErrors` and unwind
all items already created for that launch. Optional failures appear in state and
diagnostics; no mandatory icon missing without a usable error signal.

Support querying capabilities before constructing a detailed native option set.
Cross-platform configuration uses per-platform closures/builders so macOS-only
text is not accidentally submitted as a required Windows feature. Typed options
may be representable on every target; attempting them on the wrong host is an
explicit error. Document this design rather than littering callers with `cfg`.

### 7.4 Resource ownership and event flow

One `TrayHost` owns registries, native RAII objects, popup windows, images,
event ingress, observers and workers. It belongs to the application GPUI host,
separate from a particular root view and from portable `Runtime`.

1. Application builder moves specifications/handlers into this owner after
   validation. Do not require callback-bearing specifications to be `Clone`.
2. Native callbacks copy compact owned data and wake an existing foreground
   driver. They do not run user commands/closures inside FFI or worker callbacks.
3. Foreground dispatch resolves registration generation and item/menu identity,
   drops invalid/stale events, and invokes commands or local callbacks without
   holding a registry borrow or lock.
4. A menu delegate may read an immutable cached native snapshot synchronously.
   It can request later preparation, but cannot block waiting for the event pump.
5. Worker/model messages use bounded channels. Coalesce pointer moves and
   replaceable image/state updates; reserve admission for selections, lifecycle
   and completion events. Report saturation instead of silently losing commands.
6. Queues and callbacks do not strongly retain the runtime. Native callback
   trampolines are unregistered before their backing storage is freed.
7. Tray workers/drivers are infrastructure, not finite jobs that guarded quit
   waits for forever. During a declined/cancelled quit keep items functional.
8. At committed stopping, close event admission, dismiss menus/popups, cancel
   animation, unregister items/observers, finish worker shutdown and release
   native handles on their owning thread. Handle forced host teardown best-effort.

Document thread affinity on every public UI method. Prefer the foreground context
argument over hidden global thread dispatch. Do not add a Send service facade or
a second executor merely for convenience.

## 8. Menus, commands and interaction semantics

### 8.1 Shared menu model

Support text/action, separator, submenu, checkbox, mixed state, explicit radio
group and non-action descriptive/section items. Include enabled/visible state,
icon, menu-scoped accelerator metadata and stable identity. Preserve each native
extension in sections 4–6; report unsupported rich features explicitly.

The application model is authoritative. A native checkbox selection produces
one semantic action; project the accepted state back. If an adapter toggles a
native check immediately, reconcile it after command failure instead of leaving
it inconsistent. Radio groups must remain valid across insertion/removal,
disabled items and failed actions. Do not equate mixed state with checked.

Menu mutation rules:

- Keep one logical model per menu attachment, with revisioned native projections.
- Sharing a definition across trays does not imply attaching one native menu
  object to incompatible parents. Define explicit shared state versus copied
  model semantics; default to independent attachment instances.
- Keep item identity stable through label/state updates and index changes.
- Queue destructive layout changes safely during modal native tracking; apply
  allowed state changes immediately only on backends proven to support them.
- Keep selected-item payloads alive until all native callbacks are drained.
- Prepare a usable cached menu before it opens; asynchronously loaded content
  uses a disabled loading/error/retry item without blocking the shell.
- Define ownership of checked state, command availability and close-after-action
  behavior. Native menus retain native keyboard conventions.
- Preserve literal `&` and `_` correctly across Win32/DBusMenu access-key syntax;
  user-provided text is not automatically mnemonic markup.

### 8.2 Command and callback bindings

Typed Rake commands are the primary action target. Retain typed/erased bindings
privately and use existing command scheduling/results, not the devtools IPC
server. Add an internal convenience binding that generates an operation key;
applications should not invent wire request IDs to handle a menu click.

Native action identity is `(registration generation, menu generation, item ID)`.
Checks for enabled/visible/available state happen again at dispatch; a stale
native menu snapshot must not bypass application availability checks.

Support foreground callbacks for UI-specific behavior that is awkward as a
headless command. They receive event/context and GPUI access on the foreground
driver, and can start managed work. Do not require all callbacks to serialize.

`RequiresWindow` never silently creates a window. Provide explicit operations
such as show/focus/recreate main window and dispatch GPUI action to a specified
window. Built-in Quit submits guarded quit and finishes its finite callback;
it must not await the quit operation inside a task that quit is waiting to drain.

### 8.3 Event model and trigger policy

Semantic events: primary activation, secondary activation, context-menu request,
menu item selection, menu preparation/open/close/highlight (where observed),
popup state, item/host state changes, user removal, scroll, optional pointer
detail and legacy notification events.

Native detail: actual button/state/click count/modifiers, keyboard origin,
timestamp with origin/units, coordinate evidence and opaque activation token.
Unknown fields are absent; do not infer a left click for keyboard activation.

Default interaction should follow native menu conventions when an item has only
a menu. When a menu, popup and command coexist, require an explicit primary and
context policy. The same gesture cannot both open a native menu and trigger a
popup/command by accident. Describe host-controlled Linux behavior in the
effective configuration. Never promise configurable raw right-click handling
when a host consumes it.

Double-click is optional native detail. A double-click may include constituent
clicks; provide explicit native/immediate versus delayed-single-click behavior
only if needed and tested. Do not impose an invisible global click timer.

## 9. Images, text, animation and accessibility polish

### 9.1 Image sources and validation

Reuse Rake's resource pipeline for embedded/bundled assets, with a clear asset
resolution error. Accept validated RGBA representations and generated images.
Allow SVG input through an existing renderer where available; OS APIs receive
raster/native image objects rather than an assumption of native SVG support.

Separate sources:

- Portable multi-resolution image with a defined color/alpha convention.
- macOS template image or supported native/symbol image configuration.
- Linux theme name plus explicit fallback image and theme-path lifetime.
- Windows image/resource/stock-icon extension with explicit ownership.

Generate crisp representations for observed logical size and scale. Keep original
vector/high-resolution sources for regeneration. Do not assume all icons are
16 px, all macOS items are square, or all Linux hosts expose an image size.
Native-template rendering is preferable to Rake choosing a hard-coded black or
white glyph. Full-color images remain full color unless template mode is requested.

Use bounded caches keyed by content, size, scale, theme/rendering mode and
generation. Repeated identical updates are no-ops. Verify RGBA-to-AppKit/HICON/
SNI conversion with partially transparent colored test assets, not monochrome
icons that hide channel/alpha errors. Release replaced native resources after
the OS/worker no longer references them.

### 9.2 Text, tooltips, overlays and attention

Keep semantic fields separate: accessible label, descriptive native title,
tooltip, and visible inline text. A Linux `Title` is not equivalent to macOS
button text, and a tooltip is not an accessibility substitute.

Expose native overlay/attention properties on Linux; offer explicit image
composition for a badge/progress ring on other backends. State whether the
result is a native feature or rendered pixels. Neither attention nor animation
overrides user visibility/quiet preferences.

Use `Duration` and an owned animation controller. Frame timing uses the host
executor; no per-icon busy-loop thread. Coalesce stale frames, cap memory/update
rate, stop when hidden/removed/stopped, and adapt to reduced-motion settings.
The exact cap should be measured in P7. Explicit stop and restart behavior must
restore the chosen base image deterministically.

### 9.3 Accessibility and native feel

- Nonempty meaningful accessible name for actionable icon-only items.
- Distinguish disabled appearance, disabled interaction, hidden and absent.
- Preserve keyboard activation, menu arrows/type-ahead/access keys and Escape.
- Check VoiceOver, Narrator and a supported Linux accessibility stack with real
  native items/menus; native widgets alone do not establish complete accessibility.
- Preserve focus return and avoid stealing focus on passive image/state updates.
- Respect high contrast, dark/light appearance, large text and reduced motion.
- Test long/localized/RTL labels, emoji, combining marks and non-BMP characters.
- Keep hidden/user-removed items recoverable through application settings when
  removal is allowed. Never immediately re-add an item the user removed.
- Support menu-only applications without a Dock/taskbar window as far as the
  existing host allows. macOS `LSUIElement`/activation policy is a separate
  application/bundle option, not a per-tray setting.

## 10. Rich GPUI popup windows

Native menus and custom windows are distinct presentation modes. A popup hosts
normal GPUI content in a separate owned window, retaining existing services,
runtime and application theme. It is not a globally positioned in-window overlay.

Required contract:

1. Repeatable content factory or retained entity, with explicit retention policy;
   no accidental recreation of services or the application runtime.
2. Per-item popup attachment, show/toggle/close and lifecycle observation.
3. Prefer a current icon rectangle. Otherwise use the event's point hint;
   otherwise require a declared fallback such as a normal application window.
   Never call unknown geometry `(0,0)` a precise anchor.
4. Coordinate conversion handles negative monitor origins, macOS coordinate
   orientation, physical/logical scaling, menu-bar/taskbar edge, work-area
   exclusions and screen removal while open.
5. Define interactive versus nonactivating/hover popup behavior. Keyboard focus,
   Escape, click-away, deactivation and focus return need one owner and must
   work across multiple tray items. Default to one interactive tray popup at a
   time; explicitly support simultaneous popups only with a tested policy.
6. Close on owner removal/committed shutdown; define hide, host loss and display
   changes. Popup dismissal does not submit application quit.
7. Do not intercept global input using an accessibility permission or keyboard
   hook merely to implement dismissal. Prefer native window/session mechanisms.
8. On macOS compare GPUI NSPanel/window hosting, NSPopover and the newer expanded
   interface delegate. Integrate native highlighting and session cancellation
   with availability checks. Retain weak native delegates correctly.
9. On Windows use current geometry/activation context and supported focus rules;
   clamp to the monitor work area and return keyboard focus when appropriate.
10. On Wayland, arbitrary global positioning/foreground activation may be
    unavailable. Carry the XDG activation token promptly into the host adapter.
    A point hint alone cannot place an xdg_popup without an appropriate parent.
    Prove the actual GPUI/Wayland surface path before advertising anchored
    popups. Host-delegated menu remains usable when custom placement is unsupported.

Per-platform native menu custom views/owner-draw are separate advanced features.
Do not emulate a native menu with a popup while claiming identical accessibility
and keyboard behavior. Both modes get their own evidence and documentation.

## 11. Backend and dependency decision

Recommended shape: private AppKit and Win32 adapters, Linux SNI/DBusMenu through
an adequate Rust implementation, and a named optional legacy X11 adapter.
Use modules initially. A separate crate is justified only by an established
reusable backend/dependency boundary, not by the number of files.

| Candidate | Useful coverage | Adoption gate |
| --- | --- | --- |
| Existing `objc2`/AppKit bindings | Full native status/button/menu surface without another GUI runtime | Main-thread ownership; binding features and SDK/OS availability; native popup/observer safety. |
| Existing Windows bindings | Full shell/menu callbacks and resource lifecycle | Consistent `windows`/`windows-sys` boundary and versions; callback owner HWND; no duplicate GPUI message pump. |
| `tray-icon` 0.26.1 | Convenient cross-platform items, IDs, menu attachment, geometry and several native options; Linux KSNI backend | Check actual source/feature graph and missing native capabilities. It cannot define Rake's feature ceiling. Global handlers must not clash with embedded consumers. |
| `muda` 0.21.2 | Existing native menu implementation | AppKit/Win32 advanced feature gaps, accelerator/message-loop requirements, tracking and event-router ownership; Linux GTK features stay disabled unless deliberately needed. |
| `ksni` 0.3.7 | SNI/DBusMenu, scroll/attention/overlay, host lifecycle, XDG activation context | Prefer `default-features = false, features = ["async-io"]` to match existing runtime choices; own/finish service shutdown; verify missing extension hooks and menu snapshot handling. |
| Ayatana GTK library | Legacy/extension reference and alternative host integration | Obsolete upstream scope, GTK main context, deployed ABI/version, licensing and unnecessary system dependencies. Not the default. |
| Ayatana GLib replacement | GTK-free indicator/GMenu/action API | Different protocol/menu integration, library availability, GLib context and declared GPL license. Inventory/reference only unless dependency adoption is independently justified. |
| Direct X11/XEmbed adapter or existing compatible library | Legacy manager coverage | Use existing X11 facilities where sound; no unsynchronized access to GPUI's connection and no mandatory GTK loop. |

Sources: [tray-icon][tray-crate], [event limitations][tray-events],
[muda][muda-crate], [KSNI runtime][ksni-crate], [KSNI features][ksni-trait].
Versions are research snapshots; pin/resolve and record actual MSRV, licenses,
enabled features and system requirements before adopting. A wrapper patch/fork
must have a specific missing feature and upkeep plan, not merely convenience.

If a crate cannot expose a capability in sections 4–6, either extend it behind
Rake's adapter or use the underlying public API. Record the result for **every**
inventory family. Do not mark a feature supported just because the OS supports it.

## 12. Repository implementation map

Proposed initial paths, adjusted only if existing module ownership requires it:

```text
crates/rake/src/tray/
    mod.rs                 public contracts and explicit exports
    spec.rs                validated item/image/interaction specifications
    state.rs               requested/applied revisions and capability snapshots
    host.rs                application-owned registry, ingress and teardown
    menu.rs                menu model, binding and projection ownership
    popup.rs               shared popup policy and window integration
    macos.rs               AppKit adapter and native options
    windows.rs             Win32 adapter and native options
    linux/
        mod.rs             Linux selection and shared native options
        sni.rs             SNI/DBusMenu adapter and extensions
        xembed.rs          explicit optional legacy adapter
    tests/                 split by observable contracts as suites grow
```

Keep files cohesive; this list identifies responsibilities, not a requirement
to create empty modules. Backend-specific helper files can grow under their
owner. Public target modules may re-export stable Rake-owned types from private
implementation files. Do not create one crate per platform or per feature.

Affected existing files/consumers:

- `crates/rake/Cargo.toml`, root workspace dependencies and `Cargo.lock` after
  dependency selection; additive target-gated features.
- `crates/rake/src/lib.rs`, `application.rs`, `application/managed.rs`,
  `runtime.rs`, routed GPUI host binding and guarded quit integration.
- `crates/rake/src/control/commands.rs` for reusable typed menu bindings only
  where current dispatch lacks a local ergonomic entry point.
- `crates/rake-build`/resource metadata only for independent identity and image
  preparation actually needed by trays; do not enable updater policy as a shortcut.
- `docs/application.md`, `docs/platform-services.md`, a new `docs/tray.md`,
  `docs/support.md` and relevant README links.
- A focused `examples/tray` with package name `rake-tray-example`, and a
  `scripts/test-tray-native.py` or equivalent existing-style real host runner.
- External consumer tests, including renamed facade and WASM/no-default-feature
  checks. Add CI jobs only where prerequisites are real and explicitly provisioned.

Do not modify GPUI pins or vendored code without a demonstrated unavailable host
hook. If a minimal GPUI patch becomes necessary, isolate it, test both pinned
backends and document it as an explicit integration dependency.

## 13. Delivery phases and acceptance criteria

Phases organize implementation; they do not reduce the complete feature target.
All inventory entries must finish as implemented, conditional, deprecated,
host-owned/outside-scope, or explicitly infeasible with evidence. A basic icon
demo is not completion of this plan.

### P0. Prove host integration and close the adoption decisions

Work:

- Record exact dependency feature graphs, license/MSRV and native toolchain
  prerequisites; verify installed deployment minima rather than guessing.
- Spike two independently identified items on macOS, Windows, Linux SNI under
  X11 and Wayland, plus XEmbed on a legacy panel. The fixture owns IDs/resources
  explicitly and logs selected events; it is not public API scaffolding.
- Prove foreground wakeup without polling or a second GUI loop, including
  Windows TaskbarCreated reception and Linux worker cancellation/join.
- Prove keyboard activation and a menu event on each selected backend.
- Test relevant GPUI pre/fast configurations, especially AppKit raw window
  integration and native callback lifetime.
- Resolve Linux crate extension access, GMenu versus DBusMenu differences,
  activation tokens and pointer/rectangle absence.
- Probe custom GPUI popup hosting/positioning per backend. Record what Wayland
  can actually do; keep its limitations explicit.
- Audit the tables against the frozen source snapshots. Any additional public
  tray API found gets a mapping/disposition before P1 contracts are frozen.

Done when: a short native evidence report records commands, versions, item count,
events, teardown behavior, dependency choice and remaining conditional features.
Do not claim a platform passed if only a cross-compile succeeded.

### P1. Establish public contracts and a deterministic host model

Work:

- Finalize types, operation completion/cancellation semantics, capability
  vocabulary, host policies, ID validation and request limits from section 7.
- Implement the local registry/revisions/generations and a narrow fake native
  boundary for deterministic state/event/failure tests.
- Establish weak runtime access and explicit host-unavailable errors for
  headless/browser contexts.
- Validate immutable menu trees, typed bindings and interaction conflicts.
- Write external consumer examples exercising two items, dynamic updates,
  feature queries, explicit removal and runtime shutdown.

Done when: a consumer can name every returned type/error, no handle extends
runtime lifetime, stale events cannot target recreated items, and all existing
feature configurations retain their previous behavior.

### P2. Deliver multiple items, images and complete basic lifecycle

Work:

- Wire declarative `App::tray` through all builder transitions and application
  shapes. Create one application host registry and retain it without a window.
- Implement dynamic create/get/list/update/show/hide/remove with native adapters.
- Validate/render portable images, tooltips and native identity. Configure
  macOS autosave keys and Windows per-item GUID/callback IDs correctly.
- Add startup rollback and observable optional/waiting integration failure.
- Restore Windows registrations after shell recreation; handle SNI watcher/bus
  loss/reappearance and XEmbed manager changes.
- Release all native resources on correct threads, including partial failures.

Done when: two or more items remain independent through updates, a closed last
window, host loss/recovery, remove/recreate and graceful/forced teardown. Idle
items create no periodic wakeups. Basic images are verified at multiple scales.

### P3. Deliver native menus and command/event bindings

Work:

- Implement ordinary items, submenus, separators, icons, enabled/hidden states,
  check/mixed/radio state and menu-scoped accelerators/access keys.
- Dispatch through typed Rake commands; add local foreground callbacks and
  explicit GPUI action-to-window bridge where needed.
- Implement semantic activation/context/secondary events, actual native detail,
  menu lifecycle and fast cached preparation.
- Handle menu mutation during native tracking, transient command availability,
  queue backpressure and exactly-once command dispatch.
- Integrate built-in guarded Quit and proper focus return.

Done when: menu selection by pointer/keyboard performs one permitted action,
radio groups stay valid, updates during tracking are safe, declined quit leaves
every item usable, and native menus retain accessible keyboard conventions.

### P4. Complete application window/background behavior

Work:

- Add explicit repeatable standalone window factory and show/focus/recreate-main
  operation using the retained runtime and services.
- Support a background-started app opening its first normal window, routed
  apps revealing the appropriate workspace, and callbacks that need no window.
- Define popup/normal-window close versus guarded application quit; never
  silently change routed close semantics.
- Document menu-only application activation/bundle options separately.

Done when: Open/Close/Open cycles preserve app state and commands, failed mounts
remain retryable, and there is one application runtime/driver throughout.

### P5. Complete native appearance, identity and menu extensions

Work:

- macOS: item length, inline/attributed text, template/alternate/native images,
  visual versus interactive disable, user removal/restore and native persistence.
- macOS menus: subtitles, badges, alternate/indented items, state images,
  headings, palette/selection modes, tooltips and relevant native styling.
- Windows: explicit GUID/presentation/focus behavior, default menu item,
  bitmap/check images and advanced layout options where supported.
- Linux: rich tooltip, title/category/status, named icons/theme paths, overlay,
  attention, scroll, label/guide/ordering/accessibility extensions where an
  actual backend/host supports them.
- Make read-only geometry/metrics and capability explanations reliable.
- Availability-gate each newer symbol/extension and fill the implementation
  disposition column in a maintained copy of the inventory.

Done when: no explicitly requested native option silently disappears; named
features from each platform inventory either work with evidence or have a
precise documented unsupported/conditional result and reason.

### P6. Deliver rich GPUI popups

Work:

- Implement popup lifecycle, content retention, placement quality/fallbacks,
  per-item attachment and trigger arbitration from section 10.
- Integrate native focus/dismissal/highlighting, including macOS expanded
  interface sessions only on a suitable SDK/OS.
- Validate display/DPI changes, negative screen origins, all panel edges and
  activation tokens/Wayland surface constraints.
- Add interactive and explicitly nonactivating presentation modes only where
  the host supports them.

Done when: repeated open/close/switch/remove cycles have correct focus, no
orphan windows, no accidental application quit and honest anchor support on
every backend. Unsupported Wayland anchoring has a declared usable fallback.

### P7. Complete advanced and legacy coverage

Work:

- Explicit image composition for badges/progress; bounded cancellable animation.
- Finish optional pointer/hover/click detail and public-API drag/drop feasibility.
- Expose safe native menu view/owner-draw and modeless/drag options only after
  proving their resource, keyboard and accessibility behavior.
- Finish legacy Windows balloon and XEmbed message mappings, separate from
  modern OS notifications.
- Account for user-removal native termination and unavailable/private-only
  features without compromising managed quit by default.
- Publish any infeasible native hooks with source/platform evidence and a
  documented alternative, rather than shipping stub success.

Done when: every remaining inventory family has a tested implementation or
explicit documented disposition; animations/legacy callbacks cannot outlive
their item, and advanced modes meet the same lifecycle/accessibility standard.

### P8. Qualify, document and ship the complete feature set

Work:

- Complete section 14's native and consumer verification matrix.
- Finish `examples/tray`: two persistent items plus a transient item; live text/
  icon/progress; nested/check/radio menus; custom popup; failure/capability UI;
  main-window reopening; guarded quit; host-loss/recovery diagnostics.
- Write `docs/tray.md` with ownership, cancellation, native feature table,
  examples and precise Linux host prerequisites. Link from application docs.
- Update support claims only for native environments that passed their gates.
- Document evidence, known limitations, optional dependencies and a repeatable
  manual accessibility checklist. Keep production code free of design journals.

Done when: real consumers can build documented configurations, the reference
application passes the required native journeys, and every inventory row has
an honest implemented/conditional/legacy/outside-scope/unavailable disposition.

## 14. Verification and platform qualification

### 14.1 Deterministic contract and consumer tests

- Multiple IDs/duplicate rejection, removal/recreation and old handle invalidation.
- Startup partial failure cleans already-created native resources exactly once.
- Latest revision wins across delayed/out-of-order native completion and recovery.
- Unsupported requests do not mutate state; partial native failure reports reality.
- Event/menu generation validation and no ID reuse collision with queued callbacks.
- Backpressure/coalescing cannot silently lose an accepted menu action; recovery
  emits at most one action and never replays an old user activation.
- Queued callbacks can remove themselves/other items without borrow panic or UAF.
- No native handler executes while registry locks/borrows are held.
- Image validation, UTF-16/markup/access-key encoding, alpha/channel conversion,
  byte arithmetic and resource-cache invalidation.
- Menu topology validation, stable keys, checks/mixed/radio exclusivity,
  unavailable commands and mutation during simulated tracking.
- Shutdown cancellation/worker completion; denied quit preserves registrations.
- Headless/browser errors, no-default-feature build, renamed facade exports,
  both GPUI adapters and feature-isolated dependency graphs.
- Public examples compile with nameable types and ordinary imports. Memory
  doubles prove model rules, not native display, keyboard or accessibility.

Use the project test file summaries (`Covers`, `Real`, `Doubles`, `Requires`).
Default tests must not require a logged-in desktop. Real native suites use an
explicit feature/runner and fail clearly when requested prerequisites are absent.

### 14.2 Real native acceptance matrix

| Environment | Mandatory journeys |
| --- | --- |
| macOS existing minimum supported host and current available host | Multiple status items; template/full-color/text-only/combined items; light/dark/high contrast; Retina/non-Retina; autosave/user removal/restore; menu keyboard/VoiceOver; popup highlighting/focus; background app; close/reopen; declined/accepted quit. |
| macOS 27-capable SDK/runtime, when available | Expanded-interface begin/end/cancel, repeated sessions and native menu coexistence. Mark not run on older SDKs; never claim simulated coverage. |
| Windows supported current desktop versions | Multiple GUID items including overflow; keyboard activation/context menu; Narrator; fractional DPI and display changes; top/bottom/available taskbar layouts; Explorer restart; native menu focus/Escape; background HWND survival; icon/resource cleanup; legacy balloon events if enabled. |
| Linux SNI on a supported Plasma X11 and Wayland session | Multiple registrations, exported menus/state/icons, attention/overlay/scroll, activation tokens, host/bus restart, no duplicate fallback, popup placement limits and clean worker shutdown. |
| Linux Ayatana-compatible desktop | Inline label/guide/ordering and accessible descriptions only where supported; actual library versions recorded. |
| Linux desktop with no tray host | Observable unavailable/waiting state; no crashes, polling loop or invisible required-only UI with no error. |
| Legacy X11 panel/XEmbed host | Multiple docks, resizing/transparency/input, manager replacement, fallback arbitration and optional legacy messages. |
| Browser and custom headless host | Explicit unsupported GUI operations and unaffected portable runtime behavior. |

For all available native backends test two persistent plus repeated transient
items, menu edits while open, long/localized strings, display unplug, sleep/wake,
rapid updates, popup switching, removal during callbacks, forced host teardown
and sustained create/remove cycles. Do not restart a user's desktop as an
unannounced test; use a disposable test session for shell/panel restart gates.

### 14.3 Performance/resource checks

- Baseline idle CPU/wakeups with one and several static items: no periodic poll.
- Measure update throughput and latency under a burst; document coalescing and
  bounded queues, not an arbitrary responsiveness claim.
- Measure rendering/native image resource growth across repeated replacement,
  menu rebuild, popup and create/remove cycles.
- Track Windows GDI/USER handles, AppKit object lifetime and Linux bus/tasks/file
  descriptors. Verify no worker remains after explicit shutdown.
- Exercise a slow/unresponsive D-Bus host without blocking GPUI input/quit.
- Animation obeys bounds and reduced-motion policy; stopped/hidden items do not
  keep producing frames.

### 14.4 Build/check commands after implementation exists

Use the repository's exact Cargo/package configuration at implementation time.
Suggested focused commands (the `tray` feature/new test targets do not exist yet):

```sh
cargo fmt --all --check
cargo test -p rake --features tray --lib --test tray --locked
cargo test -p rake --features tray --doc --locked
cargo clippy -p rake --features tray --all-targets --locked -- -D warnings
cargo check -p rake --features tray,gpui-fast --all-targets --locked
cargo check -p rake --features tray --lib --target wasm32-unknown-unknown --locked
cargo check -p rake --no-default-features --locked
cargo check -p renamed-facade --features rake/tray --locked
cargo doc -p rake --features tray --no-deps --locked
python3 scripts/check-gpui-pins.py
```

Resolve/generate the lockfile intentionally when adding selected dependencies,
then use `--locked`. Confirm the renamed-facade package's feature wiring before
copying that illustrative invocation. Register separate real-native test commands
for each host; an `--all-targets` run is not an all-platform test.

Run relevant existing application startup/layout, commands, services, platform,
managed-work and guarded-quit suites after host ownership changes. Once focused
checks pass, run the repository's required workspace/WASM checks before declaring
completion. No build/test run is necessary merely to edit this plan.

## 15. Decisions settled by this plan and open feasibility gates

Settled direction:

- Collection-based, multiple items, application ownership and explicit removal.
- Stable names plus registration generations; native identity allocated separately.
- Shared feature union with typed native extensions and explicit capability errors.
- Native menu model, typed commands and optional foreground callbacks.
- GPUI popup windows with honest placement capability and native focus behavior.
- Rake owns cleanup; users do not keep guards in GPUI globals to keep icons alive.
- No mandatory GTK/Tokio/new GUI loop; no devtools dependency.
- No private OS API to force ordering, pinning or unavailable event delivery.

Resolve in the designated phase, without silently changing these contracts:

| Gate | Decision/evidence needed |
| --- | --- |
| P0 backend adoption | Direct versus wrapped AppKit/Win32, exact Rust Linux transport/menu backend, actual feature/license/MSRV graph. |
| P0/P6 host hooks | GPUI pre/fast window/popover access and foreground wakeup; Windows broadcast reception; public Wayland popup placement/activation route. |
| P1 request receipts | Exact completion type/names and bounded host-wait/startup policies. |
| P3 menu state | Native tracking update safety and any backend-specific safe deferral policy. |
| P5 new APIs | Compile/runtime availability for macOS menu additions and macOS 27 expanded sessions; Linux extension versions. |
| P7 advanced input | Whether public native hooks provide reliable hover/drag/drop without replacing protected host internals; record unsupported results if not. |
| P7 legacy selection | Which legacy XEmbed/balloon modes can meet the quality bar; document unsupported cases explicitly rather than conflating them with SNI/modern notifications. |

No implementation, dependency installation, shell restart, native experiment or
release qualification was performed while writing this plan. The source inventory
and repository integration analysis are research; the gates above establish runtime
behavior later.

## 16. Primary sources and research provenance

Apple documentation was supplemented with official DocC JSON at
`https://developer.apple.com/tutorials/data/documentation/appkit/<symbol>.json`
and the installed Xcode 26.6 SDK headers. Current DocC records macOS 27.0
introduction for expanded-interface delegate/session symbols. Local SDK absence
and newer online availability are deliberately recorded separately.

Linux upstream header/XML snapshots inspected:

- `iovxw/ksni` master commit `2397cc2ce535ba4e7ed77fecd94731c5239923e7`;
  `spec/{StatusNotifierItem,StatusNotifierWatcher,DBusMenu}.xml`.
- `AyatanaIndicators/libayatana-appindicator` main commit
  `836356daa20c833cbc051c77c6b4f97c94de5e61`; `src/app-indicator.h`.
- `AyatanaIndicators/libayatana-appindicator-glib` main commit
  `4fa65d8ce1a7f1a1f22cefe3d5bdd47667f188d0`;
  `src/{ayatana-appindicator.h,notification-item.xml}`.

These are upstream research snapshots, not dependency additions or promises that
every distribution ships those versions. The Freedesktop current SNI text identifies
itself as draft/version 0.1; do not infer API support from an unrelated spec index
version label. Recheck exact implementation/protocol variants during P0.

[mac-bar]: https://developer.apple.com/documentation/appkit/nsstatusbar
[mac-item]: https://developer.apple.com/documentation/appkit/nsstatusitem
[mac-button]: https://developer.apple.com/documentation/appkit/nsstatusbarbutton
[mac-removal]: https://developer.apple.com/documentation/appkit/nsstatusitem/behavior-swift.struct/removalallowed
[mac-expanded]: https://developer.apple.com/documentation/appkit/nsstatusitem/expandedinterfacedelegate
[mac-menu]: https://developer.apple.com/documentation/appkit/nsmenu
[mac-menu-item]: https://developer.apple.com/documentation/appkit/nsmenuitem
[mac-menu-delegate]: https://developer.apple.com/documentation/appkit/nsmenudelegate
[mac-popover]: https://developer.apple.com/documentation/appkit/nspopover
[win-notify]: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw
[win-data]: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw
[win-id]: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyiconidentifier
[win-rect]: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicongetrect
[win-area]: https://learn.microsoft.com/en-us/windows/win32/shell/notification-area
[win-taskbar]: https://learn.microsoft.com/en-us/windows/win32/shell/taskbar
[win-menus]: https://learn.microsoft.com/en-us/windows/win32/menurc/about-menus
[win-menu-item]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-menuiteminfow
[win-menu-info]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-menuinfo
[win-popup]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-trackpopupmenuex
[sni-item]: https://specifications.freedesktop.org/status-notifier-item/latest/status-notifier-item.html
[sni-watcher]: https://specifications.freedesktop.org/status-notifier-item/latest/status-notifier-watcher.html
[sni-xml]: https://github.com/iovxw/ksni/blob/2397cc2ce535ba4e7ed77fecd94731c5239923e7/spec/StatusNotifierItem.xml
[watcher-xml]: https://github.com/iovxw/ksni/blob/2397cc2ce535ba4e7ed77fecd94731c5239923e7/spec/StatusNotifierWatcher.xml
[dbusmenu]: https://github.com/iovxw/ksni/blob/2397cc2ce535ba4e7ed77fecd94731c5239923e7/spec/DBusMenu.xml
[aya-gtk]: https://github.com/AyatanaIndicators/libayatana-appindicator/blob/836356daa20c833cbc051c77c6b4f97c94de5e61/src/app-indicator.h
[aya-glib]: https://github.com/AyatanaIndicators/libayatana-appindicator-glib/blob/4fa65d8ce1a7f1a1f22cefe3d5bdd47667f188d0/src/ayatana-appindicator.h
[aya-project]: https://github.com/AyatanaIndicators/libayatana-appindicator-glib
[xtray]: https://specifications.freedesktop.org/systemtray/latest-single/
[xtray-opcodes]: https://specifications.freedesktop.org/systemtray/latest/messages.html
[xtray-manager]: https://specifications.freedesktop.org/systemtray/latest/manager-hints.html
[xembed]: https://specifications.freedesktop.org/xembed/latest/
[tray-crate]: https://docs.rs/tray-icon/0.26.1/tray_icon/
[tray-events]: https://docs.rs/tray-icon/0.26.1/tray_icon/enum.TrayIconEvent.html
[muda-crate]: https://docs.rs/muda/0.21.2/muda/
[ksni-crate]: https://docs.rs/ksni/0.3.7/ksni/
[ksni-trait]: https://docs.rs/ksni/0.3.7/ksni/trait.Tray.html
