import numpy as np
NUM_FIELD = 4
DEPTH = 4
def gen_perfect_tree(depth):
    nodes = []
    node_template = "DecisionTreeNode {{ use_field: {}, criteria: {}, result_class: {}, children: [{}, {}]}}"
    id = 0
    for i in range(depth):
        n = 2**i
        for j in range(n):
            id += 1
            use_field = np.random.randint(0,100) % NUM_FIELD
            f = "classify_" + str(use_field)
            r = -1
            if i == depth - 1:
                r = np.random.randint(0,10) % 2
            lc = "IS_LEAF"
            rc = "IS_LEAF"
            if i != depth -1:
                lc = 2*id -1
                rc = 2*id
            # print(use_field, f,r,lc,rc)
            node = node_template.format(use_field, f, r, lc, rc)
            nodes.append(node)
    tree = "["
    for n in nodes:
        tree += n + ",\n"
    tree += "]"
    print(tree)

def gen_samples():
    for i in range(32):
        l = []
        for i in range(4):
            l.append((np.random.randint(100) % 2) + 1)
        print(l,",", sep="")
    print("--------LABELS----------")
    l = []
    for i in range(32):
        l.append((np.random.randint(100) % 2))
    print(l)

if __name__ == "__main__":
    #gen_perfect_tree(DEPTH)
    gen_samples()