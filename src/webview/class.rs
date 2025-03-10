//! Hoists a basic NSView. In our current particular use case,
//! this is primarily used as the ContentView for a window. From there,
//! we configure an NSToolbar and WKWebview on top of them.

use std::ffi::c_void;
use std::sync::Once;
use std::ptr::null;

use block::Block;

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::foundation::{id, nil, YES, NO, NSString, NSArray, NSInteger};
use crate::webview::{WEBVIEW_DELEGATE_PTR, WebViewDelegate, mimetype::MimeType};
use crate::webview::actions::{NavigationAction, NavigationResponse};//, OpenPanelParameters};
//use crate::webview::enums::{NavigationPolicy, NavigationResponsePolicy};
use crate::utils::load;

/// Called when an `alert()` from the underlying `WKWebView` is fired. Will call over to your
/// `WebViewController`, where you should handle the event.
extern fn alert<T: WebViewDelegate>(_: &Object, _: Sel, _: id, _: id, _: id, complete: id) {
    //let alert = NSString::wrap(s).to_str();

    // @TODO: This is technically (I think?) a private method, and there's some other dance that
    // needs to be done here involving taking the pointer/invoke/casting... but this is fine for
    // now as it's being exposed purely for debugging.
    unsafe {
        let _: () = msg_send![complete, invoke];
    }

    /*unsafe {
        let ptr: usize = *this.get_ivar(WEBVIEW_DELEGATE_PTR);
        let delegate = ptr as *const T;
        (*webview).alert(alert);
    }*/

    /*let queue = dispatch::Queue::main();
    queue.exec_async(move || {
        let a = Alert::new("Subatomic", message);
        a.show();
    });*/
}

/// Fires when a message has been passed from the underlying `WKWebView`.
extern fn on_message<T: WebViewDelegate>(this: &Object, _: Sel, _: id, script_message: id) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    unsafe {
        let name = NSString::from_retained(msg_send![script_message, name]);
        let body = NSString::from_retained(msg_send![script_message, body]);
        delegate.on_message(name.to_str(), body.to_str());
    }
}

/// Fires when a custom protocol URI is requested from the underlying `WKWebView`.
extern fn start_url_scheme_task<T: WebViewDelegate>(this: &Object, _: Sel, _webview: id, task: id) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    unsafe {
        let request: id = msg_send![task, request];
        let url: id = msg_send![request, URL];

        let uri = NSString::from_retained(msg_send![url, absoluteString]);
        let uri_str = uri.to_str();

        if let Some(content) = delegate.on_custom_protocol_request(uri_str) {
            let mime = MimeType::parse(&content, uri_str);
            let nsurlresponse: id = msg_send![class!(NSURLResponse), alloc];
            let response: id = msg_send![nsurlresponse, initWithURL:url MIMEType:NSString::new(&mime)
                expectedContentLength:content.len() textEncodingName:null::<c_void>()];
            let _: () = msg_send![task, didReceiveResponse: response];

            // Send data
            let bytes = content.as_ptr() as *mut c_void;
            let data: id = msg_send![class!(NSData), alloc];
            let data: id = msg_send![data, initWithBytes:bytes length:content.len()];
            let _: () = msg_send![task, didReceiveData: data];

            // Finish
            let () = msg_send![task, didFinish];
        }
    }
}

/// Fires when a custom protocol completed the task from the underlying `WKWebView`.
extern fn stop_url_scheme_task<T: WebViewDelegate>(_: &Object, _: Sel, _webview: id, _task: id) {}

/// Fires when deciding a navigation policy - i.e, should something be allowed or not.
extern fn decide_policy_for_action<T: WebViewDelegate>(this: &Object, _: Sel, _: id, action: id, handler: usize) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    let action = NavigationAction::new(action);
    
    delegate.policy_for_navigation_action(action, |policy| unsafe {
        let handler = handler as *const Block<(NSInteger,), c_void>;
        (*handler).call((policy.into(),));
    }); 
}

/// Fires when deciding a navigation policy - i.e, should something be allowed or not.
extern fn decide_policy_for_response<T: WebViewDelegate>(this: &Object, _: Sel, _: id, response: id, handler: usize) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    let response = NavigationResponse::new(response);

    delegate.policy_for_navigation_response(response, |policy| unsafe {
        let handler = handler as *const Block<(NSInteger,), c_void>;
        (*handler).call((policy.into(),));
    });
}

/// Fires when deciding a navigation policy - i.e, should something be allowed or not.
extern fn run_open_panel<T: WebViewDelegate>(this: &Object, _: Sel, _: id, params: id, _: id, handler: usize) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    delegate.run_open_panel(params.into(), move |urls| unsafe {
        let handler = handler as *const Block<(id,), c_void>;

        match urls {
            Some(u) => {
                let nsurls: NSArray = u.iter().map(|s| {
                    let s = NSString::new(s);
                    msg_send![class!(NSURL), URLWithString:&*s]
                }).collect::<Vec<id>>().into();

                (*handler).call((nsurls.into(),));
            },

            None => { (*handler).call((nil,)); }
        }
    });
}

/// Called when a download has been initiated in the WebView, and when the navigation policy
/// response is upgraded to BecomeDownload. Only called when explicitly linked since it's a private
/// API.
#[cfg(feature = "webview-downloading-macos")]
extern fn handle_download<T: WebViewDelegate>(this: &Object, _: Sel, download: id, suggested_filename: id, handler: usize) {
    let delegate = load::<T>(this, WEBVIEW_DELEGATE_PTR);

    let handler = handler as *const Block<(objc::runtime::BOOL, id), c_void>; 
    let filename = NSString::from_retained(suggested_filename);

    delegate.run_save_panel(filename.to_str(), move |can_overwrite, path| unsafe {
        if path.is_none() {
            let _: () = msg_send![download, cancel];
        }

        let path = NSString::new(&path.unwrap());
        
        (*handler).call((match can_overwrite {
            true => YES,
            false => NO
        }, path.into()));
    });
}

/// Registers an `NSViewController` that we effectively turn into a `WebViewController`. Acts as
/// both a subclass of `NSViewController` and a delegate of the held `WKWebView` (for the various
/// varieties of delegates needed there).
pub fn register_webview_class() -> *const Class {
    static mut VIEW_CLASS: *const Class = 0 as *const Class;
    static INIT: Once = Once::new();

    INIT.call_once(|| unsafe {
        let superclass = class!(WKWebView);
        let decl = ClassDecl::new("RSTWebView", superclass).unwrap();
        VIEW_CLASS = decl.register();
    });

    unsafe { VIEW_CLASS }
}

/// Registers an `NSViewController` that we effectively turn into a `WebViewController`. Acts as
/// both a subclass of `NSViewController` and a delegate of the held `WKWebView` (for the various
/// varieties of delegates needed there).
pub fn register_webview_delegate_class<T: WebViewDelegate>() -> *const Class {
    static mut VIEW_CLASS: *const Class = 0 as *const Class;
    static INIT: Once = Once::new();

    INIT.call_once(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("RSTWebViewDelegate", superclass).unwrap();

        decl.add_ivar::<usize>(WEBVIEW_DELEGATE_PTR);

        // WKNavigationDelegate
        decl.add_method(sel!(webView:decidePolicyForNavigationAction:decisionHandler:), decide_policy_for_action::<T> as extern fn(&Object, _, _, id, usize));
        decl.add_method(sel!(webView:decidePolicyForNavigationResponse:decisionHandler:), decide_policy_for_response::<T> as extern fn(&Object, _, _, id, usize));

        // WKScriptMessageHandler
        decl.add_method(sel!(userContentController:didReceiveScriptMessage:), on_message::<T> as extern fn(&Object, _, _, id));
 
        // Custom protocol handler
        decl.add_method(sel!(webView:startURLSchemeTask:), start_url_scheme_task::<T> as extern fn(&Object, Sel, id, id));
        decl.add_method(sel!(webView:stopURLSchemeTask:), stop_url_scheme_task::<T> as extern fn(&Object, Sel, id, id));

        // WKUIDelegate
        decl.add_method(sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:), alert::<T> as extern fn(&Object, _, _, id, _, _));
        decl.add_method(sel!(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:), run_open_panel::<T> as extern fn(&Object, _, _, id, _, usize));
        
        // WKDownloadDelegate is a private class on macOS that handles downloading (saving) files.
        // It's absurd that this is still private in 2020. This probably couldn't get into the app
        // store, so... screw it, feature-gate it.
        #[cfg(feature = "webview-downloading-macos")]
        decl.add_method(sel!(_download:decideDestinationWithSuggestedFilename:completionHandler:), handle_download::<T> as extern fn(&Object, _, id, id, usize));

        VIEW_CLASS = decl.register();
    });

    unsafe { VIEW_CLASS }
}
