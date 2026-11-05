//! AppKit owns pagination, font fallback, the print panel and PDF output.
//!
//! Call only on the main executor, OUTSIDE an App/Window update: runOperation pumps
//! Cocoa events and may synchronously re-enter GPUI. No editor entities are held here.

use super::msg;
use crate::platform::printing::{Options, Prepared};
use orlo_utils::objc::{class, pool, sel, Id, NsString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static PRINT_OPEN: AtomicBool = AtomicBool::new(false);
struct PrintGuard;
impl Drop for PrintGuard {
    fn drop(&mut self) {
        PRINT_OPEN.store(false, Ordering::Release);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Size,
}

struct Owned(Id);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: these objects each came from alloc/init or copy and live on the main thread.
        unsafe { msg::<unsafe extern "C" fn(Id, Id)>()(self.0, sel(c"release")) }
    }
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSPrintSaveJob: Id;
    static NSPrintSpoolJob: Id;
    static NSPrintJobSavingURL: Id;
    static NSFontAttributeName: Id;
    static NSForegroundColorAttributeName: Id;
}

/// A native print panel, or a PDF destination (used by the disposable native QA harness).
/// PDF always uses NSPrintSaveJob: it can never send a physical print job.
pub enum Destination<'a> {
    Panel,
    Pdf(&'a Path),
}

/// False means cancellation or an AppKit-reported failure; neither changes the source.
pub fn run(title: &str, source: &str, options: Options, destination: Destination<'_>) -> Result<bool, String> {
    run_prepared(title, &Prepared { text: source.into(), styles: Vec::new() }, options, false, destination)
}

pub fn run_prepared(
    title: &str,
    source: &Prepared,
    options: Options,
    page_setup: bool,
    destination: Destination<'_>,
) -> Result<bool, String> {
    options.validate()?;
    pool(|| {
        // SAFETY: all signatures match the AppKit headers (64-bit CGFloat/NSUInteger/BOOL).
        // Every temporary object lives through runOperation and is released after it returns.
        unsafe {
            let id: unsafe extern "C" fn(Id, Id) -> Id = msg();
            let flag: unsafe extern "C" fn(Id, Id) -> u8 = msg();
            let set: unsafe extern "C" fn(Id, Id, Id) = msg();
            let set_flag: unsafe extern "C" fn(Id, Id, u8) = msg();
            let set_float: unsafe extern "C" fn(Id, Id, f64) = msg();
            if flag(class(c"NSThread"), sel(c"isMainThread")) == 0 {
                return Err("Printing must run on the main thread".into());
            }
            if !id(class(c"NSPrintOperation"), sel(c"currentOperation")).is_null() {
                return Err("Another native print operation is already open".into());
            }
            if PRINT_OPEN.swap(true, Ordering::AcqRel) {
                return Err("Another native print operation is already open".into());
            }
            let _guard = PrintGuard;
            let info = Owned(id(id(class(c"NSPrintInfo"), sel(c"sharedPrintInfo")), sel(c"copy")));
            if info.0.is_null() {
                return Err("macOS could not create print settings".into());
            }
            for selector in [c"setLeftMargin:", c"setRightMargin:", c"setTopMargin:", c"setBottomMargin:"] {
                set_float(info.0, sel(selector), 36.);
            }
            if page_setup {
                let layout = id(class(c"NSPageLayout"), sel(c"pageLayout"));
                let modal: unsafe extern "C" fn(Id, Id, Id) -> isize = msg();
                if modal(layout, sel(c"runModalWithPrintInfo:"), info.0) != 1 {
                    return Ok(false);
                }
            }
            let paginate: unsafe extern "C" fn(Id, Id, usize) = msg();
            paginate(info.0, sel(c"setHorizontalPagination:"), 1); // fit width
            paginate(info.0, sel(c"setVerticalPagination:"), 0); // paginate height
            set_flag(info.0, sel(c"setHorizontallyCentered:"), 0);
            set_flag(info.0, sel(c"setVerticallyCentered:"), 0);
            set(info.0, sel(c"setJobDisposition:"), NSPrintSpoolJob);
            if let Destination::Pdf(path) = &destination {
                let path = path.to_str().ok_or("The PDF path is not valid Unicode")?;
                let path = NsString::new(path);
                let url: unsafe extern "C" fn(Id, Id, Id) -> Id = msg();
                let url = url(class(c"NSURL"), sel(c"fileURLWithPath:"), path.0);
                set(info.0, sel(c"setJobDisposition:"), NSPrintSaveJob);
                let dict = id(info.0, sel(c"dictionary"));
                let insert: unsafe extern "C" fn(Id, Id, Id, Id) = msg();
                insert(dict, sel(c"setObject:forKey:"), url, NSPrintJobSavingURL);
            }
            let paper: unsafe extern "C" fn(Id, Id) -> Size = msg();
            let get_float: unsafe extern "C" fn(Id, Id) -> f64 = msg();
            let printable_width = (paper(info.0, sel(c"paperSize")).width
                - get_float(info.0, sel(c"leftMargin"))
                - get_float(info.0, sel(c"rightMargin")))
            .max(1.);
            let font: unsafe extern "C" fn(Id, Id, f64, f64) -> Id = msg();
            let font = font(class(c"NSFont"), sel(c"monospacedSystemFontOfSize:weight:"), options.font_size as f64, 0.);
            let dictionary: unsafe extern "C" fn(Id, Id, Id, Id) -> Id = msg();
            let attributes =
                dictionary(class(c"NSDictionary"), sel(c"dictionaryWithObject:forKey:"), font, NSFontAttributeName);
            let measure: unsafe extern "C" fn(Id, Id, Id) -> Size = msg();
            let width = if options.wrap {
                printable_width
            } else {
                source
                    .text
                    .lines()
                    .map(|line| {
                        let text = NsString::new(line);
                        measure(text.0, sel(c"sizeWithAttributes:"), attributes).width
                    })
                    .fold(printable_width, f64::max)
                    .ceil()
                    + 1.
            };
            // TextKit 2 draws entire paragraph fragments through the page clip, which
            // retains duplicate invisible text in PDFs at wrapped page boundaries. Use
            // AppKit's TextKit 1 layout for this detached print-only view (macOS 13+).
            let init: unsafe extern "C" fn(Id, Id, u8) -> Id = msg();
            let view = Owned(init(id(class(c"NSTextView"), sel(c"alloc")), sel(c"initUsingTextLayoutManager:"), 0));
            if view.0.is_null() {
                return Err("macOS could not create the print layout".into());
            }
            let frame: unsafe extern "C" fn(Id, Id, Rect) = msg();
            frame(
                view.0,
                sel(c"setFrame:"),
                Rect { origin: Point { x: 0., y: 0. }, size: Size { width, height: 20. } },
            );
            set_flag(view.0, sel(c"setRichText:"), 0);
            set_flag(view.0, sel(c"setHorizontallyResizable:"), 0);
            set_flag(view.0, sel(c"setVerticallyResizable:"), 1);
            let set_size: unsafe extern "C" fn(Id, Id, Size) = msg();
            set_size(view.0, sel(c"setMaxSize:"), Size { width, height: f64::MAX });
            let container = id(view.0, sel(c"textContainer"));
            set_size(container, sel(c"setContainerSize:"), Size { width, height: f64::MAX });
            set_flag(container, sel(c"setWidthTracksTextView:"), 1);
            set_float(container, sel(c"setLineFragmentPadding:"), 0.);
            set(view.0, sel(c"setFont:"), font);
            set(view.0, sel(c"setTextColor:"), id(class(c"NSColor"), sel(c"blackColor")));
            set(view.0, sel(c"setBackgroundColor:"), id(class(c"NSColor"), sel(c"whiteColor")));
            let text = NsString::new(&source.text);
            set(view.0, sel(c"setString:"), text.0);
            let storage = id(view.0, sel(c"textStorage"));
            #[repr(C)]
            struct NsRange {
                location: usize,
                length: usize,
            }
            let add: unsafe extern "C" fn(Id, Id, Id, Id, NsRange) = msg();
            let color: unsafe extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id = msg();
            for span in &source.styles {
                let mut hsl = span.color;
                if hsl.s < 0.15 {
                    hsl.l = 0.;
                } else {
                    hsl.l = hsl.l.min(0.35);
                }
                let rgb: gpui::Rgba = hsl.into();
                let value = color(
                    class(c"NSColor"),
                    sel(c"colorWithCalibratedRed:green:blue:alpha:"),
                    rgb.r as f64,
                    rgb.g as f64,
                    rgb.b as f64,
                    1.,
                );
                add(
                    storage,
                    sel(c"addAttribute:value:range:"),
                    NSForegroundColorAttributeName,
                    value,
                    NsRange { location: span.range.start, length: span.range.end - span.range.start },
                );
            }
            let void: unsafe extern "C" fn(Id, Id) = msg();
            void(view.0, sel(c"sizeToFit"));
            let operation: unsafe extern "C" fn(Id, Id, Id, Id) -> Id = msg();
            let operation =
                operation(class(c"NSPrintOperation"), sel(c"printOperationWithView:printInfo:"), view.0, info.0);
            if operation.is_null() {
                return Err("macOS could not create a print operation".into());
            }
            let title = NsString::new(title);
            set(operation, sel(c"setJobTitle:"), title.0);
            set_flag(operation, sel(c"setShowsPrintPanel:"), matches!(destination, Destination::Panel) as u8);
            set_flag(operation, sel(c"setShowsProgressPanel:"), matches!(destination, Destination::Panel) as u8);
            Ok(flag(operation, sel(c"runOperation")) != 0)
        }
    })
}
