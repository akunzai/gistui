# gistui

A terminal UI for browsing, comparing, and managing GitHub Gists.

## Language

**Gist mutation**:
A change to a gist itself (create, delete, edit a file, description, star, fork, compact, upload-replace). Its async outcome belongs to no single screen — List, Gists, GistDetail, and Confirm can all launch one.
_Avoid_: screen action, background job result, apply handler

**Gist catalog**:
The publishable, cacheable collection of owned and starred Gists together with the account and enrichment metadata needed to browse them. A refresh may publish newer stages of one catalog over time, but never mixes stages from different refreshes.
_Avoid_: gist list, cache snapshot, fetch result
