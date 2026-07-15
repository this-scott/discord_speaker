terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    tls = {
      source = "hashicorp/tls"
      version = "~> 4.0"
    }
  }
}

provider "aws" {
  region  = "us-east-1"
}

// generate key pair for ssh
resource "tls_private_key" "speaker" {
  algorithm = "RSA"
  rsa_bits  = 4096
}

resource "aws_key_pair" "speaker" {
  key_name   = "discord_speaker"
  public_key = tls_private_key.speaker.public_key_openssh
}

resource "local_file" "speaker_pem" {
  content         = tls_private_key.speaker.private_key_pem
  filename        = "${path.module}/discord_speaker.pem"
  file_permission = "0400"
}

resource "aws_instance" "speaker" {
    ami = "ami-08bee9c1b63b637da"

    instance_type = "t4g.nano"
    vpc_security_group_ids = [aws_security_group.speaker.id]
    key_name      = aws_key_pair.speaker.key_name

    user_data = templatefile("${path.module}/user-data.sh.tftpl", {
        discord_token         = var.discord_token
        spotify_client_id     = var.spotify_client_id
        spotify_client_secret = var.spotify_client_secret
        redirect_uri          = var.redirect_uri
        bind_addr             = var.bind_addr
    })
    user_data_replace_on_change = true

    tags = {
      Name = "discord_speaker"
    }
}

resource "aws_security_group" "speaker" {
  name        = "speaker-sg"
  description = "web + ssh for discord speaker"

  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "udp"          # HTTP/3, Caddy binds 443/udp
    cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]  # tighten to your IP/32 if you can
  }
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}