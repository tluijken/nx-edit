//! A CustomTone browse page for the HX Stomp, saved logged out on 2026-11-13 and trimmed to the
//! listing: the tone count, ten tones and the pagination.

use nx_customtone::{Query, Sort, Tone, browse_url, parse_browse};

fn page() -> nx_customtone::BrowsePage {
    parse_browse(include_str!("fixtures/browse-hx_stomp-rating-p1.html")).unwrap()
}

#[test]
fn the_first_page_by_rating_matches_the_site() {
    let url = browse_url(&Query { page: 1, sort: Sort::Rating, descending: true, search: String::new() });
    assert_eq!(url, "https://line6.com/customtone/browse/hx_stomp/1/?sort=rating&sort_dir=desc&search_term=");
}

#[test]
fn a_search_goes_to_the_search_pages_with_its_terms_url_encoded() {
    let url = browse_url(&Query { page: 3, sort: Sort::Downloads, descending: false, search: "iron maiden & co".into() });
    assert_eq!(url, "https://line6.com/customtone/search/hx_stomp/3/?sort=thecount&sort_dir=asc&search_term=iron%20maiden%20%26%20co");
}

#[test]
fn a_page_lists_its_ten_tones_and_where_it_is() {
    let page = page();
    assert_eq!(page.tones.len(), 10);
    assert_eq!((page.page, page.pages, page.total), (1, 304, 3031));
}

#[test]
fn a_tone_carries_what_the_site_shows() {
    let first = &page().tones[0];
    assert_eq!(
        (first.id, first.name.as_str(), first.author.as_str(), first.date.as_str(), first.downloads),
        (4088879, "BE100 CH1+2", "bsd512", "10/26/18", 25115)
    );
    assert_eq!(first.style, "Mainly rock and blues, but good for lots of stuff.");
    assert!(first.comment.starts_with("Dual Amp Patch using the Placater Clean"));
    assert!(first.band.is_empty());
}

#[test]
fn band_and_guitarist_are_read_when_given() {
    let tones = page().tones;
    let soundgarden = tones.iter().find(|t| t.id == 4822042).unwrap();
    assert_eq!(soundgarden.band, "Soundgarden");
    let maiden: &Tone = tones.iter().find(|t| t.band == "Iron Maiden").unwrap();
    assert_eq!(maiden.guitarist, "Adrian Smith/Dave Murray");
}

#[test]
fn a_page_that_is_not_a_listing_is_an_error() {
    assert!(parse_browse("<html><body>Maintenance</body></html>").is_err());
}

#[test]
fn a_search_page_reads_like_a_browse_page() {
    let page = parse_browse(include_str!("fixtures/search-rectifier-downloads-p1.html")).unwrap();
    assert_eq!((page.page, page.pages, page.total), (1, 2, 13));
    assert_eq!(page.tones.len(), 10);
    assert_eq!(page.tones[0].downloads, 6145);
    assert!(page.tones.windows(2).all(|w| w[0].downloads >= w[1].downloads));
}
