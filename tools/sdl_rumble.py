#!/usr/bin/env python3
"""Simule un jeu SDL3 : liste les manettes vues par SDL et envoie un rumble à chacune."""
import ctypes
import time

sdl = ctypes.CDLL("libSDL3.so.0")
sdl.SDL_GetGamepads.restype = ctypes.POINTER(ctypes.c_uint32)
for f in ("SDL_GetGamepadNameForID", "SDL_GetGamepadPathForID", "SDL_GetError"):
    getattr(sdl, f).restype = ctypes.c_char_p
sdl.SDL_OpenGamepad.restype = ctypes.c_void_p
sdl.SDL_RumbleGamepad.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_uint16, ctypes.c_uint32]

assert sdl.SDL_Init(0x2000), sdl.SDL_GetError()  # SDL_INIT_GAMEPAD
sdl.SDL_PumpEvents()
time.sleep(0.3)
sdl.SDL_PumpEvents()
count = ctypes.c_int()
ids = sdl.SDL_GetGamepads(ctypes.byref(count))
for i in range(count.value):
    path = sdl.SDL_GetGamepadPathForID(ids[i]).decode()
    print(f"SDL voit : {sdl.SDL_GetGamepadNameForID(ids[i]).decode()} ({path})")
    pad = sdl.SDL_OpenGamepad(ids[i])
    ok = sdl.SDL_RumbleGamepad(pad, 0x8000, 0x4000, 400)
    print(f"  rumble 0x8000/0x4000 400 ms -> {'ok' if ok else sdl.SDL_GetError().decode()}")
    time.sleep(0.8)
