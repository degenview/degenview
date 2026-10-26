use eframe::egui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCommand {
    NewTab,
    OpenSettings,
    SaveCurrentLayout,
    /// File ▸ Close Tab.
    CloseTab,
    /// Window ▸ Move Tab to New Window, for the window in front.
    MoveTabToNewWindow,
    /// Window ▸ Merge All Windows, into the window in front.
    MergeAllWindows,
    SelectNextTab,
    SelectPreviousTab,
    /// ⌘Q, the Dock's Quit or a logout, held back while the app asks about unsaved scripts.
    Quit,
    /// Edit ▸ Cut, Copy, Paste, Go to Definition and Rename Symbol…: for the Script Manager's
    /// code editor, whenever it has the keyboard.
    EditCut,
    EditCopy,
    EditPaste,
    GoToDefinition,
    RenameSymbol,
}

/// What the Edit menu can do right now. Everything is off unless a Script Manager editor has the
/// keyboard; an item that is off is grey, and AppKit then leaves its key to the window, so ⌘C in
/// a chart or a text field elsewhere behaves exactly as it did before the menu existed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EditMenu {
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub go_to_definition: bool,
    pub rename: bool,
}

impl EditMenu {
    fn bits(self) -> u8 {
        u8::from(self.cut)
            | u8::from(self.copy) << 1
            | u8::from(self.paste) << 2
            | u8::from(self.go_to_definition) << 3
            | u8::from(self.rename) << 4
    }

    #[cfg(any(target_os = "macos", test))]
    fn from_bits(bits: u8) -> Self {
        Self {
            cut: bits & 1 != 0,
            copy: bits & 2 != 0,
            paste: bits & 4 != 0,
            go_to_definition: bits & 8 != 0,
            rename: bits & 16 != 0,
        }
    }
}

static EDIT_MENU: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Tells the Edit menu what is available; the app does it every frame.
pub fn set_edit_menu(menu: EditMenu) {
    EDIT_MENU.store(menu.bits(), std::sync::atomic::Ordering::Release);
}

#[cfg(any(target_os = "macos", test))]
fn edit_menu() -> EditMenu {
    EditMenu::from_bits(EDIT_MENU.load(std::sync::atomic::Ordering::Acquire))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::NativeCommand;
    use eframe::egui;
    use objc2::ffi;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, ProtocolObject, Sel};
    use objc2::sel;
    use objc2_app_kit::{
        NSAccessibility, NSApplication, NSApplicationDelegate, NSEventModifierFlags, NSMenu,
        NSMenuItem, NSWindow,
    };
    use objc2_foundation::{MainThreadMarker, NSString, ns_string};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};

    static COMMANDS: Mutex<Vec<NativeCommand>> = Mutex::new(Vec::new());
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();
    /// Whether a terminate request is held for the app to answer (see `intercept_quit`).
    static INTERCEPT_QUIT: AtomicBool = AtomicBool::new(false);

    extern "C-unwind" fn new_window_for_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::NewTab);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    /// `NSApplicationTerminateReply`: with the Script Manager open, the app asks before it goes;
    /// it quits itself once the answer is in. Otherwise AppKit terminates as it always did.
    extern "C-unwind" fn should_terminate(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) -> usize {
        const TERMINATE_CANCEL: usize = 0;
        const TERMINATE_NOW: usize = 1;
        if !INTERCEPT_QUIT.load(Ordering::Acquire) {
            return TERMINATE_NOW;
        }
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::Quit);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
        TERMINATE_CANCEL
    }

    pub fn intercept_quit(on: bool) {
        INTERCEPT_QUIT.store(on, Ordering::Release);
    }

    extern "C-unwind" fn open_settings(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::OpenSettings);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    extern "C-unwind" fn save_current_layout(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::SaveCurrentLayout);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    extern "C-unwind" fn close_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        push(NativeCommand::CloseTab);
    }

    extern "C-unwind" fn move_tab_to_new_window(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        push(NativeCommand::MoveTabToNewWindow);
    }

    extern "C-unwind" fn merge_all_windows(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        push(NativeCommand::MergeAllWindows);
    }

    extern "C-unwind" fn select_next_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        push(NativeCommand::SelectNextTab);
    }

    extern "C-unwind" fn select_previous_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        push(NativeCommand::SelectPreviousTab);
    }

    extern "C-unwind" fn edit_cut(_d: &AnyObject, _s: Sel, _sender: Option<&AnyObject>) {
        push(NativeCommand::EditCut);
    }

    extern "C-unwind" fn edit_copy(_d: &AnyObject, _s: Sel, _sender: Option<&AnyObject>) {
        push(NativeCommand::EditCopy);
    }

    extern "C-unwind" fn edit_paste(_d: &AnyObject, _s: Sel, _sender: Option<&AnyObject>) {
        push(NativeCommand::EditPaste);
    }

    extern "C-unwind" fn go_to_definition(_d: &AnyObject, _s: Sel, _sender: Option<&AnyObject>) {
        push(NativeCommand::GoToDefinition);
    }

    extern "C-unwind" fn rename_symbol(_d: &AnyObject, _s: Sel, _sender: Option<&AnyObject>) {
        push(NativeCommand::RenameSymbol);
    }

    /// `validateMenuItem:`: the Edit items follow [`super::EditMenu`]; every other item the
    /// delegate owns is always available.
    extern "C-unwind" fn validate_menu_item(
        _delegate: &AnyObject,
        _selector: Sel,
        item: Option<&AnyObject>,
    ) -> Bool {
        let Some(item) = item else {
            return Bool::YES;
        };
        // SAFETY: `item` is the NSMenuItem AppKit is validating; `action` returns a selector.
        let action: Option<Sel> = unsafe { objc2::msg_send![item, action] };
        let menu = super::edit_menu();
        let enabled = match action {
            Some(action) if action == sel!(editCut:) => menu.cut,
            Some(action) if action == sel!(editCopy:) => menu.copy,
            Some(action) if action == sel!(editPaste:) => menu.paste,
            Some(action) if action == sel!(goToDefinition:) => menu.go_to_definition,
            Some(action) if action == sel!(renameSymbol:) => menu.rename,
            _ => true,
        };
        Bool::new(enabled)
    }

    fn push(command: NativeCommand) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(command);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    /// Centres the close/minimise/zoom buttons of every tab window in a title bar `height` points
    /// tall. The buttons sit for AppKit's own 28 pt bar, so under a taller strip they would ride
    /// high. AppKit lays the bar out again on resize and when leaving full screen, so this runs
    /// every frame and writes only what differs. Windows without full-size content (Settings,
    /// Alerts) keep their native title bars untouched.
    pub fn center_traffic_lights(height: f64) {
        use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        for window in NSApplication::sharedApplication(mtm).windows().iter() {
            let style = window.styleMask();
            if !style.contains(NSWindowStyleMask::FullSizeContentView)
                || style.contains(NSWindowStyleMask::FullScreen)
            {
                continue;
            }
            // AppKit drags the window from its title-bar band before egui sees the press, which
            // would carry a tab with it. The strip moves the window itself (StartDrag).
            if window.isMovable() {
                window.setMovable(false);
            }
            let (Some(close), Some(minimize), Some(zoom)) = (
                window.standardWindowButton(NSWindowButton::CloseButton),
                window.standardWindowButton(NSWindowButton::MiniaturizeButton),
                window.standardWindowButton(NSWindowButton::ZoomButton),
            ) else {
                continue;
            };
            // button → title bar → the container AppKit pins to the window's top edge.
            // SAFETY: both are plain view-hierarchy reads on the main thread, on views the
            // window owns and that stay alive for this call.
            let container = unsafe { close.superview().and_then(|bar| bar.superview()) };
            let Some(container) = container else {
                continue;
            };
            let top = window.frame().size.height;
            let wanted = NSRect::new(
                NSPoint::new(container.frame().origin.x, top - height),
                NSSize::new(container.frame().size.width, height),
            );
            if container.frame() != wanted {
                container.setFrame(wanted);
            }
            for button in [&close, &minimize, &zoom] {
                let frame = button.frame();
                let y = ((height - frame.size.height) / 2.0).round();
                if (frame.origin.y - y).abs() > 0.5 {
                    button.setFrameOrigin(NSPoint::new(frame.origin.x, y));
                }
            }
        }
    }

    pub fn install(ctx: &egui::Context) {
        let _ = CONTEXT.set(ctx.clone());
        INSTALLED.get_or_init(|| {
            let mtm = MainThreadMarker::new().expect("AppKit setup must run on the main thread");
            let app = NSApplication::sharedApplication(mtm);
            let delegate = app
                .delegate()
                .expect("winit must install its application delegate before app creation");
            let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
            let delegate_object: &AnyObject = delegate_protocol.as_ref();
            install_delegate_actions(delegate_object.class());
            install_menus(&app, mtm, delegate_object);
            // Tabs are drawn by the app (`ui::tab_bar`); keep AppKit from adding its own.
            NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
        });
    }

    fn install_delegate_actions(delegate_class: &'static AnyClass) {
        let implementation = unsafe {
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>) -> usize,
                objc2::runtime::Imp,
            >(should_terminate)
        };
        let added = unsafe {
            ffi::class_addMethod(
                delegate_class as *const AnyClass as *mut AnyClass,
                sel!(applicationShouldTerminate:),
                implementation,
                c"Q@:@".as_ptr(),
            )
        };
        assert!(
            added.as_bool(),
            "delegate already implements applicationShouldTerminate:"
        );
        install_action(delegate_class, sel!(newWindowForTab:), new_window_for_tab);
        install_action(delegate_class, sel!(openSettings:), open_settings);
        install_action(
            delegate_class,
            sel!(saveCurrentLayout:),
            save_current_layout,
        );
        install_action(delegate_class, sel!(closeTab:), close_tab);
        install_action(
            delegate_class,
            sel!(moveTabToNewWindow:),
            move_tab_to_new_window,
        );
        install_action(delegate_class, sel!(mergeAllWindows:), merge_all_windows);
        install_action(delegate_class, sel!(selectNextTab:), select_next_tab);
        install_action(
            delegate_class,
            sel!(selectPreviousTab:),
            select_previous_tab,
        );
        install_action(delegate_class, sel!(editCut:), edit_cut);
        install_action(delegate_class, sel!(editCopy:), edit_copy);
        install_action(delegate_class, sel!(editPaste:), edit_paste);
        install_action(delegate_class, sel!(goToDefinition:), go_to_definition);
        install_action(delegate_class, sel!(renameSymbol:), rename_symbol);
        let validate = unsafe {
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>) -> Bool,
                objc2::runtime::Imp,
            >(validate_menu_item)
        };
        // BOOL is `bool` ("B") on Apple silicon and `signed char` ("c") on Intel.
        #[cfg(target_arch = "aarch64")]
        let signature = c"B@:@";
        #[cfg(not(target_arch = "aarch64"))]
        let signature = c"c@:@";
        let added = unsafe {
            ffi::class_addMethod(
                delegate_class as *const AnyClass as *mut AnyClass,
                sel!(validateMenuItem:),
                validate,
                signature.as_ptr(),
            )
        };
        assert!(
            added.as_bool(),
            "delegate already implements validateMenuItem:"
        );
    }

    fn install_action(
        delegate_class: &'static AnyClass,
        selector: Sel,
        action: extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>),
    ) {
        let implementation = unsafe {
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>),
                objc2::runtime::Imp,
            >(action)
        };
        let added = unsafe {
            ffi::class_addMethod(
                delegate_class as *const AnyClass as *mut AnyClass,
                selector,
                implementation,
                c"v@:@".as_ptr(),
            )
        };
        assert!(added.as_bool(), "delegate already implements {selector}");
    }

    pub fn take_commands() -> Vec<NativeCommand> {
        COMMANDS
            .lock()
            .map(|mut commands| std::mem::take(&mut *commands))
            .unwrap_or_default()
    }

    fn install_menus(app: &NSApplication, mtm: MainThreadMarker, delegate: &AnyObject) {
        let menubar = app.mainMenu().unwrap_or_else(|| NSMenu::new(mtm));

        // winit creates the standard application menu. Add Settings there instead of appending
        // another top-level DegenView menu next to it.
        let app_menu =
            if let Some(existing) = menubar.itemAtIndex(0).and_then(|item| item.submenu()) {
                existing
            } else {
                let item = NSMenuItem::new(mtm);
                let menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("DegenView"));
                item.setSubmenu(Some(&menu));
                menubar.insertItem_atIndex(&item, 0);
                menu
            };
        remove_about_item(&app_menu);
        let settings = menu_item(
            mtm,
            ns_string!("Settings…"),
            Some(sel!(openSettings:)),
            ns_string!(","),
        );
        unsafe { settings.setTarget(Some(delegate)) };
        if let Some(existing) = app_menu.itemWithTitle(ns_string!("Settings…")) {
            unsafe {
                existing.setAction(Some(sel!(openSettings:)));
                existing.setTarget(Some(delegate));
            }
            existing.setKeyEquivalent(ns_string!(","));
        } else {
            app_menu.insertItem_atIndex(&settings, 0);
        }

        let file_item = NSMenuItem::new(mtm);
        let file_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("File"));
        let new_tab = menu_item(
            mtm,
            ns_string!("New Tab"),
            Some(sel!(newWindowForTab:)),
            ns_string!("t"),
        );
        unsafe { new_tab.setTarget(Some(delegate)) };
        file_menu.addItem(&new_tab);
        let close_tab = menu_item(
            mtm,
            ns_string!("Close Tab"),
            Some(sel!(closeTab:)),
            ns_string!("w"),
        );
        unsafe { close_tab.setTarget(Some(delegate)) };
        file_menu.addItem(&close_tab);
        let save_layout = menu_item(
            mtm,
            ns_string!("Save"),
            Some(sel!(saveCurrentLayout:)),
            ns_string!("s"),
        );
        unsafe { save_layout.setTarget(Some(delegate)) };
        file_menu.addItem(&save_layout);
        file_item.setSubmenu(Some(&file_menu));
        menubar.addItem(&file_item);

        // Edit: the code editor's clipboard and navigation. Items are grey (and their keys left to
        // the window) unless a Script Manager editor has the keyboard; see `EditMenu`.
        let edit_item = NSMenuItem::new(mtm);
        let edit_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("Edit"));
        for (title, selector, key) in [
            (ns_string!("Cut"), sel!(editCut:), ns_string!("x")),
            (ns_string!("Copy"), sel!(editCopy:), ns_string!("c")),
            (ns_string!("Paste"), sel!(editPaste:), ns_string!("v")),
        ] {
            let item = menu_item(mtm, title, Some(selector), key);
            unsafe { item.setTarget(Some(delegate)) };
            edit_menu.addItem(&item);
        }
        edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
        // Xcode's chords: F12 and F2 are media keys on a Mac keyboard. The editor answers the
        // F-keys itself.
        for (title, selector, key) in [
            (
                ns_string!("Go to Definition"),
                sel!(goToDefinition:),
                ns_string!("j"),
            ),
            (
                ns_string!("Rename Symbol…"),
                sel!(renameSymbol:),
                ns_string!("e"),
            ),
        ] {
            let item = menu_item(mtm, title, Some(selector), key);
            item.setKeyEquivalentModifierMask(
                NSEventModifierFlags::Control | NSEventModifierFlags::Command,
            );
            unsafe { item.setTarget(Some(delegate)) };
            edit_menu.addItem(&item);
        }
        edit_item.setSubmenu(Some(&edit_menu));
        menubar.addItem(&edit_item);

        let window_item = NSMenuItem::new(mtm);
        let window_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("Window"));
        window_menu.addItem(&menu_item(
            mtm,
            ns_string!("Minimize"),
            Some(sel!(performMiniaturize:)),
            ns_string!("m"),
        ));
        window_menu.addItem(&NSMenuItem::separatorItem(mtm));
        let move_tab = menu_item(
            mtm,
            ns_string!("Move Tab to New Window"),
            Some(sel!(moveTabToNewWindow:)),
            ns_string!(""),
        );
        unsafe { move_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&move_tab);
        let merge_all = menu_item(
            mtm,
            ns_string!("Merge All Windows"),
            Some(sel!(mergeAllWindows:)),
            ns_string!(""),
        );
        unsafe { merge_all.setTarget(Some(delegate)) };
        window_menu.addItem(&merge_all);
        let next_tab = menu_item(
            mtm,
            ns_string!("Show Next Tab"),
            Some(sel!(selectNextTab:)),
            ns_string!(""),
        );
        unsafe { next_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&next_tab);
        let previous_tab = menu_item(
            mtm,
            ns_string!("Show Previous Tab"),
            Some(sel!(selectPreviousTab:)),
            ns_string!(""),
        );
        unsafe { previous_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&previous_tab);
        window_item.setSubmenu(Some(&window_menu));
        menubar.addItem(&window_item);
        app.setWindowsMenu(Some(&window_menu));
        app.setMainMenu(Some(&menubar));
    }

    /// Drops winit's "About DegenView": the stock panel has nothing to show, About lives in
    /// Settings.
    fn remove_about_item(menu: &NSMenu) {
        let about = (0..menu.numberOfItems()).find(|index| {
            menu.itemAtIndex(*index)
                .is_some_and(|item| item.title().to_string().starts_with("About"))
        });
        if let Some(index) = about {
            menu.removeItemAtIndex(index);
        }
    }

    fn menu_item(
        mtm: MainThreadMarker,
        title: &NSString,
        selector: Option<Sel>,
        key: &NSString,
    ) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), title, selector, key)
        };
        // VoiceOver names menu items from their title; make it explicit.
        item.setAccessibilityTitle(Some(title));
        item
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::NativeCommand;
    use eframe::egui;

    pub fn install(_ctx: &egui::Context) {}

    pub fn take_commands() -> Vec<NativeCommand> {
        Vec::new()
    }

    pub fn intercept_quit(_on: bool) {}

    pub fn center_traffic_lights(_height: f64) {}
}

/// Keeps the macOS window buttons centred in a title bar `height` points tall (see
/// `platform::center_traffic_lights`); nothing to do elsewhere.
pub fn center_traffic_lights(height: f32) {
    platform::center_traffic_lights(f64::from(height));
}

pub fn install(ctx: &egui::Context) {
    platform::install(ctx);
}

pub fn take_commands() -> Vec<NativeCommand> {
    platform::take_commands()
}

/// While on, a terminate request becomes `NativeCommand::Quit` instead of ending the app, so
/// the app can ask about unsaved scripts first. It ends itself once the answer is in.
pub fn intercept_quit(on: bool) {
    platform::intercept_quit(on);
}

/// The mouse cursor on the whole desktop, in egui points (what `ViewportInfo::outer_rect` uses),
/// or `None` where the platform does not tell (Wayland, X11 for now). A tab being dragged out of
/// its window needs it: the window that follows the cursor moves under it, so a position derived
/// from window-relative pointer events would chase its own tail.
#[cfg(target_os = "macos")]
pub fn global_cursor(_pixels_per_point: f32) -> Option<egui::Pos2> {
    use std::ffi::c_void;

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    // SAFETY: `CGEventCreate(NULL)` returns a +1 event describing the current mouse state (or
    // null); `CGEventGetLocation` reads it in display points with a top-left origin, matching
    // winit's window coordinates; the event is released once.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let location = CGEventGetLocation(event);
        CFRelease(event);
        Some(egui::pos2(location.x as f32, location.y as f32))
    }
}

#[cfg(windows)]
pub fn global_cursor(pixels_per_point: f32) -> Option<egui::Pos2> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }

    let mut point = Point { x: 0, y: 0 };
    // SAFETY: `point` is a valid out pointer for the call.
    let ok = unsafe { GetCursorPos(&mut point) };
    (ok != 0 && pixels_per_point > 0.0).then(|| {
        egui::pos2(
            point.x as f32 / pixels_per_point,
            point.y as f32 / pixels_per_point,
        )
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn global_cursor(_pixels_per_point: f32) -> Option<egui::Pos2> {
    None
}

/// Whether the primary mouse button is held right now, according to the OS, or `None` where it
/// cannot say. A window hidden mid-drag loses focus and egui forgets its pressed buttons, which
/// would end the drag early.
#[cfg(target_os = "macos")]
pub fn primary_button_down() -> Option<bool> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const LEFT_BUTTON: u32 = 0;
    // SAFETY: a plain query of global mouse state with documented constant arguments.
    Some(unsafe { CGEventSourceButtonState(COMBINED_SESSION_STATE, LEFT_BUTTON) })
}

#[cfg(windows)]
pub fn primary_button_down() -> Option<bool> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    const VK_LBUTTON: i32 = 0x01;
    // SAFETY: a plain query of global key state. The top bit means "down now".
    Some(unsafe { GetAsyncKeyState(VK_LBUTTON) } < 0)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn primary_button_down() -> Option<bool> {
    None
}

/// The picture on the system clipboard, as PNG or JPEG bytes: a copied image, or a picture file
/// copied in Finder. `None` when there is none. Image-only clipboards give the windowing layer no
/// paste event (it has no text to report), so this is read when the user asks for the picture.
#[cfg(target_os = "macos")]
pub fn clipboard_picture() -> Option<Vec<u8>> {
    picture_on(&objc2_app_kit::NSPasteboard::generalPasteboard())
}

#[cfg(not(target_os = "macos"))]
pub fn clipboard_picture() -> Option<Vec<u8>> {
    None
}

#[cfg(target_os = "macos")]
fn picture_on(board: &objc2_app_kit::NSPasteboard) -> Option<Vec<u8>> {
    use objc2_foundation::NSString;
    for kind in ["public.png", "public.jpeg"] {
        if let Some(data) = board.dataForType(&NSString::from_str(kind)) {
            return Some(data.to_vec());
        }
    }
    let url = board.stringForType(&NSString::from_str("public.file-url"))?;
    let path = file_url_path(&url.to_string())?;
    crate::service::image_ingest::read_file(&path).ok()
}

/// The path a `file://` URL names, with its percent escapes undone. `None` for any other URL.
pub fn file_url_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // `file:///Users/me/a.png` has an empty host; `file://localhost/...` is the same place.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut input = path.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next()?;
            let low = input.next()?;
            let hex = |digit: u8| (digit as char).to_digit(16);
            bytes.push((hex(high)? * 16 + hex(low)?) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(std::path::PathBuf::from)
}

#[cfg(test)]
mod edit_menu_tests {
    use super::*;

    #[test]
    fn the_menu_survives_its_trip_through_an_atomic() {
        for bits in 0..32u8 {
            assert_eq!(EditMenu::from_bits(bits).bits(), bits);
        }
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;

    #[test]
    fn a_file_url_names_its_path_and_nothing_else_does() {
        assert_eq!(
            file_url_path("file:///Users/me/My%20Pictures/a%C3%A9.png"),
            Some(std::path::PathBuf::from("/Users/me/My Pictures/aé.png"))
        );
        assert_eq!(
            file_url_path("file://localhost/tmp/x.png"),
            Some(std::path::PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(file_url_path("https://example.com/a.png"), None);
        assert_eq!(file_url_path("file://example.com/a.png"), None);
        assert_eq!(file_url_path("file:///bad%2"), None);
        assert_eq!(file_url_path("file:///bad%zz"), None);
    }

    /// A pasteboard of the test's own: the user's clipboard is not touched.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_picture_on_a_pasteboard_is_read_back_and_other_things_are_not() {
        use objc2_app_kit::NSPasteboard;
        use objc2_foundation::{NSData, NSString};
        let board = NSPasteboard::pasteboardWithUniqueName();
        let png = b"\x89PNG\r\n\x1a\nnot really a picture, only its signature";
        board.clearContents();
        assert!(picture_on(&board).is_none(), "an empty pasteboard has none");
        board.setString_forType(
            &NSString::from_str("just text"),
            &NSString::from_str("public.utf8-plain-text"),
        );
        assert!(picture_on(&board).is_none(), "text is not a picture");
        board.setData_forType(
            Some(&NSData::with_bytes(png)),
            &NSString::from_str("public.png"),
        );
        assert_eq!(picture_on(&board).as_deref(), Some(&png[..]));
    }
}
