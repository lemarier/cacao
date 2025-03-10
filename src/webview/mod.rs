//! Wraps `WKWebView` across all platforms.
//!
//! Wraps a number of different classes/delegates/controllers into one
//! useful interface. This encompasses...
//!
//! - `WKWebView`
//! - `WKUIDelegate`
//! - `WKScriptMessageHandler`
//!
//! This is, thankfully, a pretty similar class across platforms.
//!
//! ### WebView is not available for tvOS
//! Apple does not ship `WKWebView` on tvOS, and as a result this control is not provided on that
//! platform.

use core_graphics::geometry::CGRect;

use objc_id::ShareId;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use crate::foundation::{id, nil, YES, NO, NSString};
use crate::geometry::Rect;
use crate::layout::{Layout, LayoutAnchorX, LayoutAnchorY, LayoutAnchorDimension};
use crate::layer::Layer;
use crate::utils::properties::ObjcProperty;

mod actions;
pub use actions::*;

mod config;
pub use config::WebViewConfig;

mod enums;
pub use enums::*;

pub(crate) mod class;
use class::{register_webview_class, register_webview_delegate_class};
//pub(crate) mod process_pool;

mod mimetype;
mod traits;
pub use traits::WebViewDelegate;

pub(crate) static WEBVIEW_DELEGATE_PTR: &str = "rstWebViewDelegatePtr";

fn allocate_webview(
    mut config: WebViewConfig,
    objc_delegate: Option<&Object>
) -> id {
    unsafe {
        // Not a fan of this, but we own it anyway, so... meh.
        let handlers = std::mem::take(&mut config.handlers);
        let protocols = std::mem::take(&mut config.protocols);
        let configuration = config.into_inner();
        
        if let Some(delegate) = &objc_delegate {
            // Technically private!
            #[cfg(feature = "webview-downloading-macos")]
            let process_pool: id = msg_send![configuration, processPool]; 
            
            // Technically private!
            #[cfg(feature = "webview-downloading-macos")]
            let _: () = msg_send![process_pool, _setDownloadDelegate:*delegate];

            let content_controller: id = msg_send![configuration, userContentController];
            for handler in handlers {
                let name = NSString::new(&handler);
                let _: () = msg_send![content_controller, addScriptMessageHandler:*delegate name:&*name];
            }

            for protocol in protocols {
                let name = NSString::new(&protocol);
                let _: () = msg_send![configuration, setURLSchemeHandler:*delegate forURLScheme:&*name];
            }
        }

        let zero: CGRect = Rect::zero().into();
        let webview_alloc: id = msg_send![register_webview_class(), alloc];
        let webview: id = msg_send![webview_alloc, initWithFrame:zero configuration:configuration];

        #[cfg(target_os = "macos")]
        let _: () = msg_send![webview, setWantsLayer:YES];

        let _: () = msg_send![webview, setTranslatesAutoresizingMaskIntoConstraints:NO];

        if let Some(delegate) = &objc_delegate {
            let _: () = msg_send![webview, setNavigationDelegate:*delegate];
            let _: () = msg_send![webview, setUIDelegate:*delegate];
        }

        webview
    }
}

pub struct WebView<T = ()> {
    /// An internal flag for whether an instance of a View<T> is a handle. Typically, there's only
    /// one instance that should have this set to `false` - if that one drops, we need to know to
    /// do some extra cleanup.
    pub is_handle: bool,

    /// A pointer to the Objective-C runtime view controller.
    pub objc: ObjcProperty,

    /// A reference to the underlying CALayer.
    pub layer: Layer,

    /// We need to store the underlying delegate separately from the `WKWebView` - this is a where
    /// we do so.
    pub objc_delegate: Option<ShareId<Object>>,

    /// A pointer to the delegate for this view.
    pub delegate: Option<Box<T>>,

    /// A pointer to the Objective-C runtime top layout constraint.
    pub top: LayoutAnchorY,

    /// A pointer to the Objective-C runtime leading layout constraint.
    pub leading: LayoutAnchorX,

    /// A pointer to the Objective-C runtime left layout constraint.
    pub left: LayoutAnchorX,

    /// A pointer to the Objective-C runtime trailing layout constraint.
    pub trailing: LayoutAnchorX,

    /// A pointer to the Objective-C runtime right layout constraint.
    pub right: LayoutAnchorX,

    /// A pointer to the Objective-C runtime bottom layout constraint.
    pub bottom: LayoutAnchorY,

    /// A pointer to the Objective-C runtime width layout constraint.
    pub width: LayoutAnchorDimension,

    /// A pointer to the Objective-C runtime height layout constraint.
    pub height: LayoutAnchorDimension,

    /// A pointer to the Objective-C runtime center X layout constraint.
    pub center_x: LayoutAnchorX,

    /// A pointer to the Objective-C runtime center Y layout constraint.
    pub center_y: LayoutAnchorY
}

impl Default for WebView {
    fn default() -> Self {
        WebView::new(WebViewConfig::default())
    }
}

impl WebView {
    /// An internal initializer method for very common things that we need to do, regardless of
    /// what type the end user is creating.
    ///
    /// This handles grabbing autolayout anchor pointers, as well as things related to layering and
    /// so on. It returns a generic `WebView<T>`, which the caller can then customize as needed.
    pub(crate) fn init<T>(view: id) -> WebView<T> {
        unsafe {
            let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints:NO];

            #[cfg(target_os = "macos")]
            let _: () = msg_send![view, setWantsLayer:YES];
        }

        WebView {
            is_handle: false,
            delegate: None,
            objc_delegate: None,
            top: LayoutAnchorY::top(view),
            left: LayoutAnchorX::left(view),
            leading: LayoutAnchorX::leading(view),
            right: LayoutAnchorX::right(view),
            trailing: LayoutAnchorX::trailing(view),
            bottom: LayoutAnchorY::bottom(view),
            width: LayoutAnchorDimension::width(view),
            height: LayoutAnchorDimension::height(view),
            center_x: LayoutAnchorX::center(view),
            center_y: LayoutAnchorY::center(view),
            
            layer: Layer::wrap(unsafe {
                msg_send![view, layer]
            }),

            objc: ObjcProperty::retain(view),
        }
    }
    

    /// Returns a default `WebView`, suitable for customizing and displaying.
    pub fn new(config: WebViewConfig) -> Self {
        let view = allocate_webview(config, None);
        WebView::init(view)
    }
}

impl<T> WebView<T> where T: WebViewDelegate + 'static {
    /// Initializes a new WebView with a given `WebViewDelegate`. This enables you to respond to events
    /// and customize the view as a module, similar to class-based systems.
    pub fn with(config: WebViewConfig, delegate: T) -> WebView<T> {
        let delegate = Box::new(delegate);

        let objc_delegate = unsafe {
            let objc_delegate: id = msg_send![register_webview_delegate_class::<T>(), new];
            let ptr: *const T = &*delegate;
            (&mut *objc_delegate).set_ivar(WEBVIEW_DELEGATE_PTR, ptr as usize);
            ShareId::from_ptr(objc_delegate)
        };

        let view = allocate_webview(config, Some(&objc_delegate));
        let mut view = WebView::init(view);

        &delegate.did_load(view.clone_as_handle()); 
        view.delegate = Some(delegate);
        view
    }
}

impl<T> WebView<T> {
    /// An internal method that returns a clone of this object, sans references to the delegate or
    /// callback pointer. We use this in calling `did_load()` - implementing delegates get a way to
    /// reference, customize and use the view but without the trickery of holding pieces of the
    /// delegate - the `View` is the only true holder of those.
    pub(crate) fn clone_as_handle(&self) -> WebView {
        WebView {
            delegate: None,
            top: self.top.clone(),
            leading: self.leading.clone(),
            left: self.left.clone(),
            right: self.right.clone(),
            is_handle: true,
            trailing: self.trailing.clone(),
            bottom: self.bottom.clone(),
            width: self.width.clone(),
            height: self.height.clone(),
            center_x: self.center_x.clone(),
            center_y: self.center_y.clone(),
            layer: self.layer.clone(),
            objc: self.objc.clone(),
            objc_delegate: None
        }
    }

    /// Given a URL, instructs the WebView to load it.
    //  @TODO: Make this take Url instead? Fine for testing now I suppose.
    pub fn load_url(&self, url: &str) {
        let url = NSString::new(url);

        self.objc.with_mut(|obj| unsafe {
            let u: id = msg_send![class!(NSURL), URLWithString:&*url];
            let request: id = msg_send![class!(NSURLRequest), requestWithURL:u];
            let _: () = msg_send![&*obj, loadRequest:request];
        });
    }

    /// Go back in history, if possible.
    pub fn go_back(&self) {
        self.objc.with_mut(|obj| unsafe {
            let _: () = msg_send![&*obj, goBack];
        });
    }

    /// Go forward in history, if possible.
    pub fn go_forward(&self) {
        self.objc.with_mut(|obj| unsafe {
            let _: () = msg_send![&*obj, goForward];
        });
    }
}

impl<T> Layout for WebView<T> {
    fn with_backing_node<F: Fn(id)>(&self, handler: F) {
        self.objc.with_mut(handler);
    }

    fn get_from_backing_node<F: Fn(&Object) -> R, R>(&self, handler: F) -> R {
        self.objc.get(handler)
    }

    /// Currently, this is a noop. Theoretically there is reason to support this, but in practice
    /// I've never seen it needed... but am open to discussion.
    fn add_subview<V: Layout>(&self, _: &V) {}
}

impl<T> std::fmt::Debug for WebView<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebView ({:p})", self)
    }
}

impl<T> Drop for WebView<T> {
    /// A bit of extra cleanup for delegate callback pointers.
    fn drop(&mut self) {
        if !self.is_handle {
            self.objc.with_mut(|obj| unsafe {
                let _: () = msg_send![&*obj, setNavigationDelegate:nil];
                let _: () = msg_send![&*obj, setUIDelegate:nil];
            });
            
            self.remove_from_superview();
        }
    }
}
