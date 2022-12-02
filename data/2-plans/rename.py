# Rename a Narwhal result to standard format

from os import listdir, remove
from os.path import exists, isfile, join
import json


def load_json(file):
    with open(file) as f:
        return json.load(f)


def get_name(data):
    consensus = data["input"]['consensus'].lower()
    batch_size = data["input"]['parameters']['batch_size']
    transaction_size = data["input"]['parameters']['transaction_size']

    batch_size = batch_size // transaction_size
    rate = data["input"]['client']["injection_rate"]

    return f"result-{consensus}-ts{transaction_size}-bs{batch_size}-ir{rate}"


if __name__ == "__main__":
    mypath = "."
    fs = [f for f in listdir(mypath) if isfile(join(mypath, f))]
    fs = [join(mypath, f) for f in fs if f.startswith('result')]

    def is_narwhal(f): return 'Narwhal' == f[1]["input"]["consensus"]

    fs = ((get_name(data), n, data)
          for (n, data) in filter(is_narwhal, [(f, load_json(f)) for f in fs]))

    for name, origin, data in fs:

        remove(origin)

        # if file exists, append "--n"
        if exists(name + ".json"):
            i = 1
            while exists(name + f"--{i}" + ".json"):
                i += 1

            name = name + f"--{i}"
        file_name = name + ".json"
        with open(file_name, 'w') as f:
            json.dump(data, f, indent=2)
