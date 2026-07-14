terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

provider "aws" {
  region  = "us-east-1"
}

module "ec2_instance" {
    source  = "terraform-aws-modules/ec2-instance/aws"

    name = "speaker-instance"

    instance_type = "t4g.nano"
    monitoring    = true
    key_name      = "discord_speaker"

    tags = {
        Environment = "dev"
    }
}