variable "discord_token" {
  type = string
  sensitive = true
}

variable "spotify_client_id" {
  type = string
  sensitive = true
}

variable "spotify_client_secret" {
  type = string
  sensitive = true
}

variable "redirect_uri" {
  type = string
}

variable "bind_addr" {
  type = string
}