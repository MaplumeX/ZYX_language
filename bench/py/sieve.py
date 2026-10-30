# 埃氏筛：数组读写
def sieve(n):
    flags = [True] * n
    flags[0] = False
    flags[1] = False
    i = 2
    while i * i < n:
        if flags[i]:
            j = i * i
            while j < n:
                flags[j] = False
                j += i
        i += 1
    count = 0
    for f in flags:
        if f:
            count += 1
    return count

total = 0
for round in range(5):
    total += sieve(2000000)
print(total)
