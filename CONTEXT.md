# gistui

A terminal UI for browsing, comparing, and managing GitHub Gists.

## Language

**Gist mutation**:
A change to a gist itself (create, delete, edit a file, description, star, fork, compact, upload-replace). Its async outcome belongs to no single screen — List, Gists, GistDetail, and Confirm can all launch one.
_Avoid_: screen action, background job result, apply handler
