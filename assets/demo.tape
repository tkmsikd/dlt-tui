Output demo.gif

Set FontSize 14
Set Width 1200
Set Height 640
Set Padding 10
Set TypingSpeed 60ms

Type "./dlt-tui demo.dlt"
Enter
Sleep 2.5s

# Scroll with vim keys
Type "jjjjjjjjjj"
Sleep 800ms
Type "G"
Sleep 1s
Type "g"
Sleep 800ms

# Filter: warnings and above
Type "l"
Sleep 600ms
Type "W"
Sleep 400ms
Enter
Sleep 1.5s

# Regex search for CAN messages
Type "/"
Sleep 600ms
Type "CAN"
Sleep 400ms
Enter
Sleep 1.5s

# Open detail view with hex dump
Enter
Sleep 2s
Type "j"
Sleep 600ms
Type "j"
Sleep 1s
Type "q"
Sleep 800ms

# Clear filters, toggle delta time
Type "C"
Sleep 1s
Type "t"
Sleep 2s
