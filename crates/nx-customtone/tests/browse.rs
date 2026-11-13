//! Hand-written CustomTone listings for the HX Stomp with invented tones, in the markup the parser
//! reads from line6.com: the tone count, ten tones and the pagination.

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
    assert_eq!((page.page, page.pages, page.total), (1, 247, 2468));
}

#[test]
fn a_tone_carries_what_the_site_shows() {
    let first = &page().tones[0];
    assert_eq!(
        (first.id, first.name.as_str(), first.author.as_str(), first.date.as_str(), first.downloads),
        (9100417, "Copper Kettle CH1+2", "fuzzwright", "3/14/21", 18240)
    );
    assert_eq!(first.style, "Mostly blues and rock, fine for plenty else.");
    assert!(first.comment.starts_with("Two-amp patch built on a clean"));
    assert!(first.band.is_empty());
}

#[test]
fn band_and_guitarist_are_read_when_given() {
    let tones = page().tones;
    let saints = tones.iter().find(|t| t.id == 9100522).unwrap();
    assert_eq!(saints.band, "The Gravel Saints");
    let lantern: &Tone = tones.iter().find(|t| t.band == "Lantern Drive").unwrap();
    assert_eq!(lantern.guitarist, "Mara Quill/Ostin Vey");
}

#[test]
fn a_page_that_is_not_a_listing_is_an_error() {
    assert!(parse_browse("<html><body>Maintenance</body></html>").is_err());
}

#[test]
fn a_search_page_reads_like_a_browse_page() {
    let page = parse_browse(include_str!("fixtures/search-rectifier-downloads-p1.html")).unwrap();
    assert_eq!((page.page, page.pages, page.total), (1, 2, 14));
    assert_eq!(page.tones.len(), 10);
    assert_eq!(page.tones[0].downloads, 4810);
    assert!(page.tones.windows(2).all(|w| w[0].downloads >= w[1].downloads));
}

#[test]
fn downloads_are_read_from_the_download_box_not_the_comment() {
    let html = include_str!("fixtures/browse-hx_stomp-rating-p1.html")
        .replacen("Two-amp patch built on", "Thanks for 1000 downloads! Two-amp patch built on", 1);
    let first = &parse_browse(&html).unwrap().tones[0];
    assert_eq!(first.downloads, 18240);
}

#[test]
fn a_tone_without_an_id_is_skipped_not_the_page() {
    let html = include_str!("fixtures/browse-hx_stomp-rating-p1.html").replacen("/customtone/tone/9100417/", "/customtone/tone/x/", 2);
    let page = parse_browse(&html).unwrap();
    assert_eq!(page.tones.len(), 9);
    assert!(page.tones.iter().all(|t| t.id != 9100417));
}
