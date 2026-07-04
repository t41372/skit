import argparse

DEFAULT_N = 10

parser = argparse.ArgumentParser()
parser.add_argument("--n", type=int, default=DEFAULT_N)
args = parser.parse_args([])
print(args.n)
