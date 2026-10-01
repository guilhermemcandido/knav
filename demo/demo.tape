# The README demo. Needs a cluster with the knav-demo Deployment and a release build:
#   cargo build --release && vhs demo/demo.tape

Output demo/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1680
Set Height 900
Set Padding 12
Set Theme "Dracula"
Set TypingSpeed 90ms

# A fresh config with icons off: VHS can't capture the images terminals draw them with.
Env XDG_CONFIG_HOME "/tmp/knav-demo"

Hide
Type "rm -rf /tmp/knav-demo && mkdir -p /tmp/knav-demo/knav && printf '[ui]\nicons = false\n' > /tmp/knav-demo/knav/config.toml && clear && ./target/release/knav"
Enter
Show

# Loading screen, then Home.
Sleep 5s

Type ":pods"
Sleep 500ms
Enter
Sleep 2.5s

# Info panel on a pod: usage, status, containers.
Down 2
Sleep 500ms
Type "i"
Sleep 3s
Type "i"
Sleep 800ms

# Related objects.
Type "R"
Sleep 3s
Escape
Sleep 1s

# Everything that needs a look.
Type "!"
Sleep 3.5s
Escape
Sleep 800ms

# A Deployment's history: what rolling back would change.
Type ":deploy"
Sleep 500ms
Enter
Sleep 1.5s
Type "/knav-demo"
Sleep 400ms
Enter
Sleep 1s
Type "v"
Sleep 1.5s
Down@700ms 2
Sleep 3.5s
Escape
Sleep 800ms

# Sidebar.
Type "b"
Sleep 2.5s
