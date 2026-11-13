# CustomTone (line6.com)

CustomTone has no API; `nx-customtone` reads the pages and endpoints a browser uses. Seen
2026-11-13, signed out and signed in, before and after registering an HX Stomp on the account.

| What | Request | Notes |
|---|---|---|
| Listing | `GET /customtone/browse/hx_stomp/{page}/?sort=…&sort_dir=asc\|desc&search_term=` | 10 tones per page; `search_term` is ignored here |
| Search | `GET /customtone/search/hx_stomp/{page}/?sort=…&sort_dir=…&search_term=…` | the same page layout |
| Sort values | `name song band guitarist amp posted style thecount rating` | `posted` is date, `thecount` downloads |
| Sign in | `POST /account/login.html` form `action=login redirect=/customtone/ l_email_user l_pass submit=Login` | refused: stays on `login.html` with `( Error: 257)` |
| Signed in? | a page contains `?action=logout` | the site's own Sign Out link |
| May download? | the download button handler opens `#ct_reg_req` ("Registration required") | absent once a product is registered |
| Download | `GET /customtone/tone/deliver/{id}/` | `Content-Type: application/hlx`, `Content-Disposition: inline;filename="<name>.hlx"`, the `.hlx` JSON |
| Sign out | `GET …?action=logout` | nx-edit also drops the client and its cookies |

The listing's tone block (`div.tone`) holds the tone link (`/customtone/tone/{id}/`), the author
(`/customtone/profile/{name}/`), `TONE NAME:`, `BAND:`, `GUITARIST:`, `SONG:` lines, a style line,
the comment (`div.comment`), the date (`.date`) and `N downloads` in the download box.

The site asks that downloads be for personal use only. nx-edit fetches one page or one tone per
click, never in bulk.
