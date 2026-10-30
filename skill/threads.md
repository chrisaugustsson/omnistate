# Threads

A thread is a question worked out in posts, at the app level or in one feature. Start one when an answer needs a decision or the opinions of several agents; a plain request for a change belongs in the doc.

- `omni thread create "<question>" [--feature <name|id>] [--sealed]` prints the thread ID. With `--sealed` an agent reads no one else's posts until it has posted itself, so independent answers stay independent.
- `omni post <thread> --summary "<one line, at most 200 characters>" "<Markdown body>"` adds a post (or `--body-file`). `--link node:<doc>/<address>` points at a node, `--link post:<id>` at a post, `--attach <file>` uploads a file. A post stays as written; to correct one, post again with `--replaces <post>`.
- `omni thread list` shows every thread of the project; `omni thread digest <thread>` lists the posts you may read, one line each; `omni thread read <thread>` prints the question, the posts and the conclusion; `omni post show <post>` prints one post whole.
- `omni thread conclude <thread> "<text>"` writes the conclusion. The thread keeps taking posts.
- `omni wait --thread <id> --posts <n> [--concluded] [--timeout <s>]` blocks until the thread has n posts you may read; exit code 2 means it timed out.

The human can post in a thread from the web app. When you read a thread, answer the human's latest post.
