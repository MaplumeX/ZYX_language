# 结构体：字段读写
class Particle:
    def __init__(self, x, y, vx, vy):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy

def run(n, steps):
    ps = []
    for i in range(n):
        ps.append(Particle(i, i * 2 % 1000, i % 7 - 3, i % 5 - 2))
    for s in range(steps):
        for p in ps:
            p.x += p.vx
            p.y += p.vy
            if p.x < 0 or p.x > 1000:
                p.vx = -p.vx
            if p.y < 0 or p.y > 1000:
                p.vy = -p.vy
    sx = 0
    sy = 0
    for p in ps:
        sx += p.x
        sy += p.y
    print(sx, sy)

run(1000, 5000)
