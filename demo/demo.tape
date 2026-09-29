# The README demo. Needs a cluster and a release build:
#   cargo build --release && vhs demo/demo.tape

Output demo/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1680
Set Height 900
Set Padding 12
Set Theme "Dracula"
Set TypingSpeed 90ms

# An empty config, so the demo shows the defaults.
Env XDG_CONFIG_HOME "/tmp/knav-demo"

Hide
Type "rm -rf /tmp/knav-demo && clear && ./target/release/knav"
Enter
Show

# Loading screen, then Home.
Sleep 5s

Type ":pods"
Sleep 500ms
Enter
Sleep 2.5s

# Info panel on a pod.
Down 2
Sleep 500ms
Type "i"
Sleep 2.5s
Type "i"
Sleep 800ms

# Related objects.
Type "R"
Sleep 3s
Escape
Sleep 1s

# Search.
Type "/store"
Sleep 600ms
Enter
Sleep 2.5s

# Sidebar.
Type "b"
Sleep 2.5s
