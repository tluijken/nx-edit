//! Signing in to line6.com, against pages saved on 2026-11-13: signed out, signed in to an account
//! without a registered product, and a refused sign-in. Nothing personal is kept in them.

use nx_customtone::{Account, account_state, login_form, parse_browse, sign_in_error};

const SIGNED_OUT: &str = include_str!("fixtures/browse-hx_stomp-rating-p1.html");
const SIGNED_IN: &str = include_str!("fixtures/signed-in-unregistered.html");
const REFUSED: &str = include_str!("fixtures/login-failed.html");

#[test]
fn the_form_sends_what_the_site_form_sends() {
    let form = login_form("me@example.com", "p&ss word");
    assert_eq!(
        form,
        [
            ("action", "login".to_owned()),
            ("redirect", "/customtone/".to_owned()),
            ("l_email_user", "me@example.com".to_owned()),
            ("l_pass", "p&ss word".to_owned()),
            ("submit", "Login".to_owned()),
        ]
    );
}

#[test]
fn a_listing_without_sign_out_is_signed_out() {
    assert_eq!(account_state(SIGNED_OUT), Account::SignedOut);
}

#[test]
fn an_account_without_a_registered_product_cannot_download() {
    assert_eq!(account_state(SIGNED_IN), Account::SignedIn { can_download: false });
}

#[test]
fn a_signed_in_listing_still_parses() {
    assert_eq!(parse_browse(SIGNED_IN).unwrap().tones.len(), 10);
}

#[test]
fn a_refused_sign_in_reports_the_site_error() {
    assert_eq!(sign_in_error(REFUSED), Some(257));
    assert_eq!(sign_in_error(SIGNED_IN), None);
}
