# The README screenshots. Needs a release build and a cluster with the knav-demo
# Deployment (a few revisions) and a grafana pod. Run: vhs demo/screenshots.tape

Output demo/screenshots/.tape.gif

Set Shell "bash"
Set FontSize 15
Set Width 1500
Set Height 760
Set Padding 12
Set Theme "Dracula"
Set TypingSpeed 40ms

# A fresh config with icons off, and an "editor" that bumps the replicas.
Env XDG_CONFIG_HOME "/tmp/knav-shots"
Env EDITOR "perl -0pi -e 's/^  replicas: 2$/  replicas: 3/m'"

Hide
Type "rm -rf /tmp/knav-shots && mkdir -p /tmp/knav-shots/knav && printf '[ui]\nicons = false\n' > /tmp/knav-shots/knav/config.toml && clear && ./target/release/knav"
Enter
Sleep 6s
Show

# Rollout history: what going back to revision 2 changes.
Type ":deploy"
Enter
Sleep 2s
Type "/knav-demo"
Enter
Sleep 1s
Type "v"
Sleep 2.5s
Down
Down
Sleep 1.5s
Screenshot "demo/screenshots/history.png"
Sleep 500ms

# The edit review, cancelled.
Escape
Sleep 800ms
Type "e"
Sleep 2.5s
Screenshot "demo/screenshots/edit.png"
Sleep 500ms
Escape
Sleep 800ms

# Problems across the cluster.
Type "!"
Sleep 3s
Screenshot "demo/screenshots/problems.png"
Sleep 500ms
Escape
Sleep 800ms

# Related objects around a pod.
Type ":pods"
Enter
Sleep 2s
Type "/grafana"
Enter
Sleep 1s
Type "R"
Sleep 3s
Screenshot "demo/screenshots/relations.png"
Sleep 500ms
Escape
Sleep 800ms

# The info panel with usage, on a pod with limits.
Type ":pods"
Enter
Sleep 1.5s
Type "/knav-demo web"
Enter
Sleep 1s
Type "i"
Sleep 5s
Screenshot "demo/screenshots/info.png"
Sleep 500ms
Type "i"
Sleep 500ms

# Permissions.
Type "P"
Sleep 1.5s
Screenshot "demo/screenshots/permissions.png"
Sleep 500ms
Escape
Sleep 500ms
Type "Q"
