"""Insert transcribed text into whatever app currently has focus."""

import time


def insert_text(text: str, mode: str, clipboard=None) -> None:
    if not text:
        return
    if clipboard is not None:
        clipboard.setText(text)
    if mode == "clipboard":
        return
    time.sleep(0.15)  # let the focused app settle after the hotkey release
    import keyboard
    if mode == "type":
        keyboard.write(text, delay=0.005)
    else:  # "paste"
        keyboard.send("ctrl+v")
