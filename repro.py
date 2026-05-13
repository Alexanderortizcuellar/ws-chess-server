import asyncio
import websockets
import json

async def test():
    uri = "ws://127.0.0.1:3000/ws"
    try:
        async with websockets.connect(uri) as websocket:
            print("Connected")
            cmd = {"type": "create_game", "time_ms": 600000}
            await websocket.send(json.dumps(cmd))
            print(f"Sent: {cmd}")
            
            # Read multiple messages
            for _ in range(5):
                try:
                    response = await asyncio.wait_for(websocket.recv(), timeout=2.0)
                    print(f"Received: {response}")
                except asyncio.TimeoutError:
                    print("Waiting...")
                except websockets.exceptions.ConnectionClosed as e:
                    print(f"Connection closed: {e}")
                    break
            
            print("Closing connection...")
    except Exception as e:
        print(f"Error: {e}")

if __name__ == "__main__":
    asyncio.run(test())
