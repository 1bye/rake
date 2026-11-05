# Native tray API disposition

This mirrors every inventory row in sections 4–6 of the [implementation plan](../.agents/plans/tray-icons.md). “Implemented” describes the adapter; it does not upgrade Windows/Linux desktop qualification. Partial rows name remaining unavailable controls explicitly. Native APIs that exist but lack a qualified host integration are not described as impossible OS features. See [usage and verification](tray.md).

## 4. macOS API inventory and mapping

### 4.1 Complete direct status bar/item surface

| Native API family | Current implementation / disposition |
| --- | --- |
| `NSStatusBar.system` / Objective-C `systemStatusBar` | Internal: system AppKit bar connection. |
| `statusItem(withLength:)` / `statusItemWithLength:` | Implemented: Collection::create, independent resources. |
| `removeStatusItem(_:)` | Implemented: remove/shutdown/Drop. |
| `NSStatusBar.isVertical`, `thickness` | Implemented: Handle::macos_metrics. |
| `NSStatusItem.statusBar` | Internal: retained native ownership. |
| `length`, `squareLength`, `variableLength`, corresponding C length constants | Implemented: macos::ItemLength. |
| `menu` | Implemented: menu attachment and explicit show_menu. |
| `button` | Internal: standard native button, no replacement view. |
| `behavior`, `NSStatusItem.Behavior`, raw-value construction | Partial: Prevent/Allow/RequestQuit; unknown/native termination flags are not exported. |
| `.removalAllowed` | Implemented: KVO visibility plus show-to-restore. |
| `.terminationOnRemoval` | Intentionally unavailable: bypasses managed quit guards; use Removal::RequestQuit. |
| `isVisible` / Objective-C `visible` | Implemented: requested visibility, native KVO events and metrics. |
| `autosaveName`, `NSStatusItem.AutosaveName` | Implemented: namespaced persistent key; ordinary teardown does not clear preferences. |
| `expandedInterfaceDelegate` | Unavailable: macOS 27 symbol absent in installed SDK/runtime qualification. |
| `expandedInterfaceSession` | Unavailable: same macOS 27 gate. |
| `NSStatusItemExpandedInterfaceDelegate.statusItem(_:didBegin:)` | Unavailable: same macOS 27 gate. |
| `statusItemDidEndExpandedInterfaceSession(_:animated:)` | Unavailable: same macOS 27 gate. |
| `NSStatusItemExpandedInterfaceSession.cancel()` | Unavailable: same macOS 27 gate. |
| `NSStatusBarButton.appearsDisabled` | Implemented: separate visual disabled flag. |
### 4.2 Inherited button/image/input capabilities relevant to trays

| Native API family | Current implementation / disposition |
| --- | --- |
| `NSButton.image`, `alternateImage`, image position/scaling; `NSImage.isTemplate` and representations | Implemented: image/alternate/template/multiple representations. Position/scaling use native leading/default layout; arbitrary policies are not exported. |
| `title`, `attributedTitle`, `alternateTitle`, font/alignment/content tint where supported | Partial: plain/attributed/alternate titles and per-run fonts/colors. Arbitrary alignment/tint controls are not exported. |
| `isEnabled`, state, highlight configuration | Implemented: disabled interaction and popup highlight; arbitrary button state styling is not exported. |
| `target`, `action`, `sendAction(on:)`, `performClick`, button double-action support | Partial: native target/action delivery and click detail. Synthetic performClick/doubleAction entry points are not exported. |
| `NSEvent` button, modifiers, timestamp, location, click count and scroll information | Partial: button, modifiers, monotonic timestamp and click count. Pointer location/scroll/hover require unsupported status-button event hooks. |
| `NSView.toolTip`, `NSAccessibility` label/help/value/role/action properties | Partial: tooltip, accessible name/help and native standard role/actions. Arbitrary accessibility role/value overrides are not exported. |
| `bounds`, `window`, coordinate conversion, backing scale, display notifications | Implemented: native rectangle, display identity, target work area and per-show conversion. |
| Public dragging-destination registration/protocols and event tracking hooks | Unavailable: public drag/hover hooks on the system-owned button have not passed host/lifetime/accessibility gates. No swizzling. |
| Effective appearance, accessibility display preferences and image changes | Native-owned template appearance; explicit composed images/playback honor GPUI reduced motion. Other appearance observers are not exported. |
### 4.3 Deprecated direct NSStatusItem APIs, explicitly accounted for

| Native API family | Current implementation / disposition |
| --- | --- |
| `action`, `doubleAction`, `target`, `sendActionOn:` | Deprecated aliases: use modern button bridge; no separate legacy API. |
| `title`, `attributedTitle`, `image`, `alternateImage`, `enabled`, `toolTip` | Deprecated aliases: use modern button appearance. |
| `highlightMode` | Deprecated: supported popup highlight uses the native button. |
| `view` | Unavailable: replacing the system status view is not the rich-content integration. Use Popup. |
| `drawStatusBarBackgroundInRect:withHighlight:` | Deprecated: system-owned background/highlight drawing. |
| `popUpStatusItemMenu:` | Deprecated: use native attachment/modern NSMenu popup. |
### 4.4 Complete menu feature families exposed by AppKit

| Native API family | Current implementation / disposition |
| --- | --- |
| `NSMenu.init(title:)`, `NSMenuItem.init(title:action:keyEquivalent:)`; coder initializers | Implemented runtime construction. Coder/archive restoration is outside descriptor ownership. |
| `addItem`, `insertItem`, `removeItem`, `removeAllItems`, `items`, `numberOfItems`, `itemChanged` | Implemented descriptor replacement/rebuild with tracking deferral; raw index mutation stays internal. |
| `item(at:)`, `item(withTag:)`, `item(withTitle:)`, all `index(of:)` / `indexOfItem` variants | Internal native lookup; public lookup uses stable IDs. |
| `setSubmenu`, `submenuAction`, `supermenu`; item `submenu`, `hasSubmenu`, `parent`, `menu` | Implemented bounded nested menus and validation. |
| Item `title`, `attributedTitle`, `subtitle`, `toolTip` | Implemented plain/attributed labels/tooltips; subtitle checks the runtime selector (14.4+). |
| `image`, `onStateImage`, `offStateImage`, `mixedStateImage`, `preferredImageVisibility`, `ImageVisibility` | Partial: normal/off/on/mixed images implemented. Preferred-image-visibility policy is not exported. |
| `state`, `NSMenu.selectedItems`, `selectionMode`, `SelectionMode.{automatic,selectAny,selectOne}` | Implemented check/radio state and availability-gated native selection mode. Application model remains authoritative. |
| `isEnabled`, `isHidden`, `isHiddenOrHasHiddenAncestor`, `autoenablesItems`, `update` | Implemented explicit enabled/hidden projection and ancestor checks; no accidental responder-chain validation. |
| `separator`, `isSeparatorItem`, `sectionHeader(title:)`, `isSectionHeader` | Implemented separator; Header projects as a disabled label. Native section-header constructor is not selected. |
| `badge`, `NSMenuItemBadge` | Implemented count/text/new/updates/alerts badge; runtime selector gate (14+). |
| `keyEquivalent`, `keyEquivalentModifierMask`, `usesUserKeyEquivalents`, `userKeyEquivalent`, `allowsAutomaticKeyEquivalentLocalization`, `allowsAutomaticKeyEquivalentMirroring`, `allowsKeyEquivalentWhenHidden` | Partial: native key equivalent/mask implemented. OS user overrides remain native defaults; hidden-item/localization/mirroring policy is not exported. |
| `isAlternate`, `indentationLevel`, `tag`, `representedObject`, `target`, `action`, `isHighlighted`, menu `highlightedItem` | Implemented alternate/indentation/stable payload/target/action and highlighted-ID events; native tags remain internal. |
| `NSMenuItem.view` | Unavailable: arbitrary native menu views need independent keyboard/accessibility/lifetime integration; use Popup. |
| Menu `font`, `minimumWidth`, `size`, `title`, `showsStateColumn`, `userInterfaceLayoutDirection` | Partial: minimum width/state column and attributed per-item fonts implemented. Menu-wide font/measurements/RTL policy are not exported. |
| `presentationStyle`, `PresentationStyle.{regular,palette}`, `palette(colors:titles:template:onSelectionChange:)` | Partial: palette presentation and selection policy gated on native selectors. Specialized color-palette callback constructor is not exported. |
| `popUp(positioning:at:in:)`, both `popUpContextMenu` variants, `cancelTracking`, `cancelTrackingWithoutAnimation` | Partial: modern popup and cancel-on-shutdown. Arbitrary confinement/placement/cancel-without-animation controls are not exported. |
| `performKeyEquivalent`, `performActionForItem` | Native key handling; native-test action fixture only. No public synthetic command bypass. |
| `propertiesToUpdate`, `NSMenu.Properties`; delegate `menuNeedsUpdate`, `numberOfItems`, `menu(_:update:at:shouldCancel:)` | Internal cached full-tree projection; no asynchronous native delegate I/O or incremental population API. |
| Delegate `menuWillOpen`, `menuDidClose`, `menu(_:willHighlight:)`, `menuHasKeyEquivalent`, `confinementRect` | Partial: open/close/highlight events. Custom key-equivalent/confinement delegates are not exported. |
| `didAddItemNotification`, `didChangeItemNotification`, `didRemoveItemNotification`, `didBeginTrackingNotification`, `didEndTrackingNotification`, `willSendActionNotification`, `didSendActionNotification` | Internal implementation mechanism; useful events have one Rake event stream. |
| `NSMenu.DidAddItemMessage`, `DidChangeItemMessage`, `DidRemoveItemMessage`, `DidBeginTrackingMessage`, `DidEndTrackingMessage`, `WillSendActionMessage`, `DidSendActionMessage` | Not separately used: typed newer notification equivalents have no additional Rake semantics. |
| `allowsContextMenuPlugIns`, `automaticallyInsertsWritingToolsItems`, item `writingToolsItems`, `importFromDeviceIdentifier` | Partial: explicit context-menu plug-ins. Writing Tools/device import are not exported and remain native defaults. |
| `menuBarVisible`, `setMenuBarVisible`, `menuBarHeight`; deprecated global menu management | Outside tray scope: global menu-bar visibility mutation is not an item option. |
| Deprecated `setTitleWithMnemonic`, older NSMenu symbols | Deprecated mnemonic API not used; explicit access-key contract is portable. |
## 5. Windows API inventory and mapping

### 5.1 Direct shell operations, structures and all tray flags

| Native API family | Current implementation / disposition |
| --- | --- |
| `Shell_NotifyIconW(NIM_ADD)` | Implemented registration. |
| `NIM_MODIFY` | Implemented update, including DPI/settings resource refresh. |
| `NIM_DELETE` | Implemented explicit/remove/partial-failure/Drop cleanup. |
| `NIM_SETFOCUS` | Implemented after native menu tracking. |
| `NIM_SETVERSION`, `NOTIFYICON_VERSION_4` | Implemented version-4 negotiation on create and Explorer restoration. |
| `NOTIFYICONDATAW.cbSize` | Internal current NOTIFYICONDATAW size. |
| `hWnd`, `uID`, `uCallbackMessage` | Internal per-item top-level callback HWND; native ID is independent from application ID. |
| `guidItem`, `NIF_GUID` | Implemented deterministic per-item GUID or windows::Presentation explicit UUID. |
| `uFlags`: `NIF_MESSAGE`, `NIF_ICON`, `NIF_TIP`, `NIF_STATE` | Implemented native flags and fixed buffers. |
| `NIF_SHOWTIP` | Implemented windows::TooltipMode Standard versus Events. |
| `hIcon` | Implemented owned HICON and premultiplied BGRA DIB conversion. |
| `szTip` | Implemented plain tooltip, validated 127 UTF-16 units. |
| `dwState`, `dwStateMask`, `NIS_HIDDEN` | Implemented requested hidden state; pin/overflow presentation is shell-owned. |
| `NIS_SHAREDICON` | Not selected: independently owned icons avoid shared-handle ownership ambiguity. |
| `uVersion` | Internal version-4 union use. |
| `NOTIFYICONIDENTIFIER.{cbSize,hWnd,uID,guidItem}` | Internal exact native identity for queries. |
| `Shell_NotifyIconGetRect` | Implemented optional exact rectangle; hidden/overflow queries may fail. |
### 5.2 Full callback/event families relevant to tray interaction

| Native API family | Current implementation / disposition |
| --- | --- |
| `NIN_SELECT`, `NIN_KEYSELECT` | Implemented semantic pointer/keyboard activation without duplicate command dispatch. |
| `WM_CONTEXTMENU` | Implemented context event; no fabricated pointer button or undefined packed position. |
| Delivered `WM_MOUSEMOVE`, left/right/middle and X-button down/up/double-click messages | Partial: delivered left/right/middle down/up/double-click details. Mouse-move/X-button streams are not exported. |
| `NIN_POPUPOPEN`, `NIN_POPUPCLOSE` | Implemented rich-tooltip request/dismiss events, distinct from pointer hover. |
| `NIN_BALLOONSHOW`, `NIN_BALLOONHIDE`, `NIN_BALLOONTIMEOUT`, `NIN_BALLOONUSERCLICK` | Implemented legacy BalloonEvent values. |
| `RegisterWindowMessageW("TaskbarCreated")` | Implemented broadcast-capable hidden top-level HWND and re-add/version negotiation. |
| `WM_SETTINGCHANGE`, `WM_THEMECHANGED`, display/DPI/session/shutdown messages as applicable | Partial: settings/display/DPI updates implemented. Theme is native-rendered. Session/shutdown remain the application host lifecycle. |
### 5.3 Legacy balloon fields and controls, fully accounted for

| Native API family | Current implementation / disposition |
| --- | --- |
| `NIF_INFO`, `szInfo`, `szInfoTitle` | Implemented LegacyBalloon title/body with UTF-16 limits. |
| `NIF_REALTIME` | Implemented realtime/discard option. |
| `dwInfoFlags`: `NIIF_NONE`, `NIIF_INFO`, `NIIF_WARNING`, `NIIF_ERROR`, `NIIF_USER` | Implemented typed balloon icons, including owned custom image. |
| `NIIF_NOSOUND`, `NIIF_LARGE_ICON`, `NIIF_RESPECT_QUIET_TIME` | Implemented silence/large-icon/quiet-time options. |
| `NIIF_ICON_MASK` | Internal mask; no independent application flag. |
| `hBalloonIcon` | Implemented separate owned custom balloon icon. |
| Union `uTimeout` | Deprecated/ignored by modern Shell policy; no fake configurable duration. |
| Empty balloon body / item removal | Implemented empty-body withdrawal/removal; no guarantee an already seen balloon becomes unseen. |
### 5.4 Native popup menu inventory

| Native API family | Current implementation / disposition |
| --- | --- |
| `CreatePopupMenu`, `DestroyMenu`; `LoadMenuW`, `LoadMenuIndirectW` | Implemented owned runtime menus. Resource-template import is not exported. |
| `InsertMenuItemW`, `SetMenuItemInfoW`, `GetMenuItemInfoW`, `RemoveMenu`, `DeleteMenu`; older append/insert/modify APIs | Implemented descriptor rebuild using modern insertion; equivalent raw mutation APIs stay internal. |
| `GetMenuItemCount`, `GetMenuItemID`, `GetSubMenu`, `GetMenuState`, `GetMenuStringW`, `IsMenu` | Internal lookups; public identity is stable Rake IDs. |
| `MENUITEMINFO` ID, submenu, text, item-data and bitmap/checkmark fields; `MIIM_*` masks | Partial: ID/submenu/text/bitmap projection implemented. Custom separate checked/unchecked bitmap fields are not exported. |
| `MFT_STRING`, `MFT_SEPARATOR`, `MFT_RADIOCHECK`; `MFS_CHECKED/UNCHECKED`, enabled/disabled/grayed/default/highlighted state | Implemented action/check/radio/separator/disabled/default states. Highlight remains native-owned. |
| `MFT_OWNERDRAW`, `WM_MEASUREITEM`, `WM_DRAWITEM`, bitmap items | Partial: native bitmap items implemented. Owner-draw callbacks are unavailable without a qualified accessibility/theme/scale bridge. |
| `MFT_MENUBREAK`, `MFT_MENUBARBREAK`, `MFT_RIGHTJUSTIFY`, `MFT_RIGHTORDER`, legacy `MFT_BITMAP` | Partial: menu/menubar breaks and right-order cascading implemented. Right-justify/obsolete bitmap styles are not exported. |
| `CheckMenuItem`, `CheckMenuRadioItem`, `EnableMenuItem`, `Set/GetMenuDefaultItem`, `HiliteMenuItem`, `SetMenuItemBitmaps` | Descriptor state/default mapping implemented; raw Hilite/independent bitmap operations are not public. |
| `TrackPopupMenu` / `TrackPopupMenuEx`, `TPMPARAMS.rcExclude`, `CalculatePopupWindowPosition` | Partial: TrackPopupMenuEx at current icon/cursor placement. Exclusion rectangles and custom positioning policy are not exported. |
| `TPM_LEFT/CENTER/RIGHTALIGN`, `TOP/VCENTER/BOTTOMALIGN`, `HORIZONTAL/VERTICAL`, `LAYOUTRTL`, `WORKAREA` | Native default alignment/work-area handling; arbitrary TPM alignment/direction controls are not exported. |
| `TPM_LEFTBUTTON/RIGHTBUTTON`, `RETURNCMD`, `NONOTIFY`, `RECURSE`, animation flags, `NOANIMATION` | Internal RETURNCMD/NONOTIFY/right-button selection. Reentrant tracking and custom animation flags are not exported. |
| `SetForegroundWindow`, post-tracking focus handling and `NIM_SETFOCUS` | Implemented native foreground/focus return sequence. |
| `EndMenu`, `GetMenuItemRect`, `MenuItemFromPoint`, `GetMenuCheckMarkDimensions`, `GetMenuInfo`/`SetMenuInfo` | Internal menu ownership; arbitrary EndMenu/item geometry/hit-test/info operations are not exported. |
| `MENUINFO` style, maximum height, background brush, context help and menu data | Unavailable custom MENUINFO presentation/brush/help options; normal native defaults are retained. |
| `MNS_AUTODISMISS`, `CHECKORBMP`, `NOCHECK`, `MODELESS`, `NOTIFYBYPOS`, `DRAGDROP` | Unavailable modeless/drag/notify-by-position modes: no established host/lifetime/accessibility bridge. Normal modal menus use return-command tracking. |
| `WM_INITMENU`, `WM_INITMENUPOPUP`, `WM_UNINITMENUPOPUP`, `WM_ENTERMENULOOP`, `WM_EXITMENULOOP`, `WM_MENUSELECT` | Native tracking is synchronous; no Windows menu lifecycle/highlight event claim. AppKit/SNI lifecycle is documented separately. |
| `WM_COMMAND`, `WM_MENUCOMMAND`, `WM_MENUCHAR`, `WM_HELP`, `WM_MENURBUTTONUP`, `WM_MENUDRAG`, `WM_MENUGETOBJECT` | Internal return-command path avoids duplicate WM_COMMAND handling. Help/right-click/drag/accessibility-object hooks are not exported. |
| Accelerator tables / `TranslateAcceleratorW` | Unavailable execution hook in the pinned GPUI message pump. Mnemonics work; accelerator setters reject, never display fake execution. |
## 6. Linux API inventory and mapping

### 6.1 All StatusNotifierItem properties, requests and signals

| Native API family | Current implementation / disposition |
| --- | --- |
| `Id` | Implemented application/item identity. |
| `Category` | Implemented linux::Category. |
| `Title` | Implemented independent title. |
| `Status`: `Passive`, `Active`, `NeedsAttention` | Implemented native activity/attention; requested hidden projects Passive. |
| `WindowId` | No linked X11 toplevel is promised; adapter exports zero. Use explicit application window operations. |
| `IconName`, `IconPixmap` | Implemented named image plus all validated ARGB pixmaps. |
| `OverlayIconName`, `OverlayIconPixmap` | Implemented native overlay fields. |
| `AttentionIconName`, `AttentionIconPixmap`, `AttentionMovieName` | Implemented attention image/name/movie fields; actual animation is host-owned. |
| `ToolTip` tuple | Implemented title/body/image; plain body is escaped before markup interpretation. |
| `ItemIsMenu`, `Menu` | Implemented exported menu and registration-fixed primary interaction. |
| `IconThemePath` (deployed extension) | Implemented named-icon theme path. |
| `Activate(x,y)` | Implemented activation event. |
| `SecondaryActivate(x,y)` | Implemented secondary activation event. |
| `ContextMenu(x,y)` | Host-owned exported DBusMenu path. No duplicate local ContextMenu popup/forced-show claim. |
| `Scroll(delta,orientation)` | Implemented orientation/delta scroll events. |
| `ProvideXdgActivationToken(token)` (deployed extension) | Implemented opaque activation token on received activation/menu events. Pinned GPUI has no tray-surface activation-token injection; centered fallback remains explicit. |
| `NewTitle`, `NewIcon`, `NewAttentionIcon`, `NewOverlayIcon`, `NewToolTip`, `NewStatus(status)` | Implemented by SNI backend property monitor; checked export errors propagate. |
### 6.2 All watcher/host lifecycle members

| Native API family | Current implementation / disposition |
| --- | --- |
| `RegisterStatusNotifierItem(service-or-object-path)` | Implemented per-item registration and watcher re-registration. |
| `RegisteredStatusNotifierItems` | Host-owned registry, not application-owned tray collection; not used as presentation proof. |
| `IsStatusNotifierHostRegistered`, `ProtocolVersion` | Internal registration checks; Require/Wait policy. |
| `StatusNotifierItemRegistered`, `StatusNotifierItemUnregistered` | Host-owned watcher signals; no duplicate public registry API. |
| `StatusNotifierHostRegistered`, deployed `StatusNotifierHostUnregistered` | Host/daemon-owned; watcher ownership observations supply useful item state. |
| `RegisterStatusNotifierHost` | Outside item scope: Rake is a client, not a desktop panel host. |
| D-Bus name ownership changes, connection loss, Properties/Introspectable interfaces | Implemented name-change recovery and D-Bus interfaces. Closed bus worker ends with offline event; whole bus replacement requires remove/recreate. |
### 6.3 All DBusMenu protocol members and item properties

| Native API family | Current implementation / disposition |
| --- | --- |
| Root properties `Version`, `TextDirection`, `Status`, `IconThemePath` | Implemented backend protocol properties; text direction remains native default. |
| `GetLayout(parentId,recursionDepth,propertyNames)` | Implemented revisioned bounded backend menu projection. |
| `GetGroupProperties`, `GetProperty` | Implemented native protocol queries. |
| `Event`, `EventGroup` | Implemented backend event dispatch; scoped generations/revisions are rechecked on foreground delivery. |
| `AboutToShow`, `AboutToShowGroup` | Implemented backend cached preparation and MenuOpened hint; no universal MenuClosed claim. |
| `ItemsPropertiesUpdated`, `LayoutUpdated` | Implemented backend model/signals; export errors are observable. |
| `ItemActivationRequested(id,timestamp)` | Not exported: host-controlled activation request is not a guaranteed forced menu opening. |
| Item `type`, `label`, `enabled`, `visible` | Implemented native label/type/visibility/availability. |
| `icon-name`, `icon-data` | Implemented native name/PNG menu images. |
| `shortcut` | Implemented host-interpreted shortcut declaration; conditional execution. |
| `toggle-type`, `toggle-state` | Implemented check/radio projection. Mixed is rejected by the selected adapter. |
| `children-display` | Implemented nested submenu projection. |
| `disposition` | Not exported: default normal disposition only. |
| `x-<vendor>-*` properties/types | Unavailable unrecognized vendor properties/types; no silent transport of arbitrary vendor data. |
### 6.4 Ayatana/AppIndicator public API variants

| Native API family | Current implementation / disposition |
| --- | --- |
| `get_type`, GObject type/cast checks; `new`, `new_with_path` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| `set_status` / `get_status`, category/ID getters | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GTK `set_icon`, `set_icon_full`, `get_icon`, `get_icon_desc`; GLib `set_icon` with description | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GTK `set_attention_icon`, `set_attention_icon_full`, getters; GLib description-taking variant | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| `set_title` / `get_title` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| `set_icon_theme_path` / getter | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| `set_label(label,guide)`, `get_label`, `get_label_guide` | Unavailable Ayatana extension: ksni 0.3.7 does not export these label/guide/order/accessibility/secondary wire fields. Standard SNI equivalents above remain available; no GTK/GLib library is linked. |
| `set_ordering_index` / getter | Unavailable Ayatana extension: ksni 0.3.7 does not export these label/guide/order/accessibility/secondary wire fields. Standard SNI equivalents above remain available; no GTK/GLib library is linked. |
| `set_secondary_activate_target` / getter | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| `set_menu` / `get_menu` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GLib `set_actions` / `get_actions` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GTK `set_tooltip_icon/title/body/full`, matching getters; GLib `set_tooltip` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GTK `build_menu_from_desktop` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| Signals `new-icon`, `new-attention-icon`, `new-status`, `new-icon-theme-path`, `new-label`, `new-tooltip`, `connection-changed`, `scroll-event`, newer `activate` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| GTK subclass virtual `fallback`, `unfallback` | No Ayatana C-library binding: use the standard SNI semantic equivalent above where available. GTK/GObject ownership, GActions/GMenu import, desktop-file menus and library fallback hooks are not part of the selected backend. |
| Native wire properties `IconAccessibleDesc`, `AttentionAccessibleDesc`, `XAyatanaLabel`, `XAyatanaLabelGuide`, `XAyatanaOrderingIndex` | Unavailable Ayatana extension: ksni 0.3.7 does not export these label/guide/order/accessibility/secondary wire fields. Standard SNI equivalents above remain available; no GTK/GLib library is linked. |
| Wire `XAyatanaSecondaryActivate`, `XAyatanaNewLabel`, `NewIconThemePath` | Unavailable Ayatana extension: ksni 0.3.7 does not export these label/guide/order/accessibility/secondary wire fields. Standard SNI equivalents above remain available; no GTK/GLib library is linked. |
### 6.5 Legacy X11 system tray and XEmbed

| Native API family | Current implementation / disposition |
| --- | --- |
| `_NET_SYSTEM_TRAY_Sn`, selection-owner lookup, `MANAGER` announcement | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `_NET_SYSTEM_TRAY_OPCODE`, `SYSTEM_TRAY_REQUEST_DOCK` | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| XEmbed `_XEMBED`, `_XEMBED_INFO`, embedding/focus/mapping messages | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `XEMBED_EMBEDDED_NOTIFY`, window activation/deactivation, focus in/out/request, traversal, modality and accelerator messages | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| Configure/map/unmap/destroy/reparent/property events | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `_NET_SYSTEM_TRAY_VISUAL`, ARGB/compositing and legacy ParentRelative behavior | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `_NET_SYSTEM_TRAY_ORIENTATION` | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `WM_NORMAL_HINTS`, base/minimum size and aspect-ratio hints | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| X pointer/button/key/enter/leave events on the embedded window | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| `_NET_SYSTEM_TRAY_MESSAGE_DATA`, `SYSTEM_TRAY_BEGIN_MESSAGE`, `SYSTEM_TRAY_CANCEL_MESSAGE` | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |
| Panel color/size/padding extensions such as `_NET_SYSTEM_TRAY_COLORS`, `_NET_SYSTEM_TRAY_ICON_SIZE`, `_NET_SYSTEM_TRAY_PADDING` | Unavailable legacy backend: no XEmbed adapter/fallback arbitration or legacy-panel qualification is installed. Use SNI on X11/Wayland. This does not imply the X11 protocol is unavailable. |

NSPopover is not used as a native container: Rake uses independent GPUI windows. Direct view-controller hosting, AppKit expanded sessions, arbitrary custom menu views, Windows owner-draw/modeless/drag menus and XEmbed legacy messages remain unavailable. These require dedicated native integration and keyboard/accessibility gates; they are not stub success paths.

Source inventory and primary links are preserved in the plan. New AppKit selectors check runtime availability; macOS 27 symbols are absent in the installed SDK. Linux backend modifications are recorded in [ksni patch notes](../vendor/ksni/RAKE-PATCHES.md).
