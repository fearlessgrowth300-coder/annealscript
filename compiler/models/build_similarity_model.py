"""Builds models/similarity.onnx: a hand-authored (not trained) ONNX graph
that computes cosine similarity between two int8[32] quantized vectors.
This is the same computation the Rust prototype did in pure Rust in Phase 3
-- it's re-expressed as a real ONNX graph so it can run through actual
ONNX Runtime instead of hand-written arithmetic.
"""
import onnx
from onnx import TensorProto, helper

DIM = 32

a = helper.make_tensor_value_info("a", TensorProto.INT8, [DIM])
b = helper.make_tensor_value_info("b", TensorProto.INT8, [DIM])
similarity = helper.make_tensor_value_info("similarity", TensorProto.FLOAT, [])

nodes = [
    helper.make_node("Cast", ["a"], ["a_i32"], to=TensorProto.INT32),
    helper.make_node("Cast", ["b"], ["b_i32"], to=TensorProto.INT32),

    helper.make_node("Mul", ["a_i32", "b_i32"], ["dot_terms"]),
    helper.make_node("ReduceSum", ["dot_terms"], ["dot"], keepdims=0),

    helper.make_node("Mul", ["a_i32", "a_i32"], ["a_sq"]),
    helper.make_node("ReduceSum", ["a_sq"], ["norm_a"], keepdims=0),
    helper.make_node("Mul", ["b_i32", "b_i32"], ["b_sq"]),
    helper.make_node("ReduceSum", ["b_sq"], ["norm_b"], keepdims=0),

    helper.make_node("Cast", ["dot"], ["dot_f"], to=TensorProto.FLOAT),
    helper.make_node("Cast", ["norm_a"], ["norm_a_f"], to=TensorProto.FLOAT),
    helper.make_node("Cast", ["norm_b"], ["norm_b_f"], to=TensorProto.FLOAT),

    helper.make_node("Mul", ["norm_a_f", "norm_b_f"], ["norm_prod"]),
    helper.make_node("Sqrt", ["norm_prod"], ["denom"]),
    helper.make_node("Add", ["denom", "eps"], ["denom_safe"]),
    helper.make_node("Div", ["dot_f", "denom_safe"], ["similarity"]),
]

eps_init = helper.make_tensor("eps", TensorProto.FLOAT, [], [1e-6])

graph = helper.make_graph(
    nodes, "quantized_similarity", [a, b], [similarity], initializer=[eps_init]
)
model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 18)])
model.ir_version = 9
onnx.checker.check_model(model)
onnx.save(model, "similarity.onnx")
print("wrote similarity.onnx")
