output "cluster_name" {
  value = aws_eks_cluster.edgeplane.name
}

output "node_group_name" {
  value = aws_eks_node_group.edgeplane.node_group_name
}
