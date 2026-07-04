import asyncio

DELAY = 0.0


async def main():
    await asyncio.sleep(DELAY)
    print("done")


asyncio.run(main())
